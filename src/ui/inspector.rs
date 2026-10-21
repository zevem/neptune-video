//! The inspector: the selected clip's values, a library item's facts, or the
//! project's settings when nothing is selected.
//!
//! Fields write into a draft while they are dragged or focused. The viewer
//! draws from the same draft, and the application turns it into one command
//! when the interaction ends, so a drag is one undo step.
mod audio;
mod clip;
mod color;
mod curve;
mod look;
mod masks;
mod motion;
mod speed;
#[cfg(test)]
mod tests;
mod text;
mod transition;
mod well;

pub use color::{Channel, key_colour_command};

use super::{
    Action, ClipDraft, ColourPick, DuckProgress, Selection, SteadyShown, TrackProgress,
    TrackRegion, controls, icon_of_media,
    kit::{self, KeyRow, KeyState, RowFit},
    tone_of_media,
    viewer::trim_number,
    widgets::{self, Kind},
};
use crate::{
    icons::{self, Icon},
    textures::Textures,
    theme::{self, Palette, metrics},
    timefmt,
};
use editor_core::{
    Asset, CanvasSettings, Clip, ClipId, Command, EffectKind, FrameRate, MediaKind, Project, Time,
};
use eframe::egui::{
    self, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    WidgetType, epaint::RectShape, pos2, vec2,
};
use uuid::Uuid;

/// Project values being edited; committed with the clip draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectDraft {
    pub name: Option<String>,
    pub canvas: Option<CanvasSettings>,
}

pub struct InspectorInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub textures: &'a Textures,
    /// Where a keyframe made here is put, read when it is made.
    pub playhead: Time,
    /// The edge of the selected clip whose transition is shown in the
    /// clip's place: the one whose marking is selected in the timeline.
    pub transition: Option<editor_core::TransitionEdge>,
    /// The audio kinds the session's FFmpeg does not run, as its probe
    /// answered; `None` while it has not, and every kind is shown as one
    /// that runs.
    pub missing: Option<&'a [EffectKind]>,
    /// The font families a title may name, once the title lane has said.
    pub families: Option<&'a [String]>,
    /// The person's brand colours, offered in every colour of a title.
    pub brand: &'a [[u8; 4]],
}

/// The parts a clip's values are shown in, one at a time. A clip is offered
/// only the parts that hold something the project accepts for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Section {
    #[default]
    Video,
    Look,
    Color,
    Speed,
    Motion,
    /// A clip on an audio track's: its playback and its audio effects.
    Audio,
    Text,
    Graphic,
    Animate,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Look => "Look",
            Self::Color => "Color",
            Self::Speed => "Speed",
            Self::Motion => "Motion",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Graphic => "Graphic",
            Self::Animate => "Animate",
        }
    }
}

/// What a parameter is called wherever it is named: the inspector's rows and
/// the timeline's keyframes.
pub fn param_name(target: editor_core::AnimTarget, key: editor_core::ParamKey) -> &'static str {
    look::label(target, key)
}

pub fn effect_title(kind: EffectKind) -> &'static str {
    look::effect_name(kind)
}

/// The looks a clip can be given, as they are named.
pub fn looks() -> &'static [(editor_core::LookPreset, &'static str)] {
    &look::PRESETS
}

/// The heading an effect is listed under, and where that heading comes
/// among the others.
pub fn effect_family(kind: EffectKind) -> (usize, &'static str) {
    let family = look::family(kind);
    let order = (look::Family::ALL.iter()).position(|listed| *listed == family);
    (order.unwrap_or(look::Family::ALL.len()), family.label())
}

/// Where a new effect of `kind` goes in `stack`: a key or a colour effect
/// where the Color section would have made it, any other at the end.
pub fn place_effect(stack: &[editor_core::Effect], kind: EffectKind) -> usize {
    if color::makes(kind) {
        color::place(stack, kind)
    } else {
        stack.len()
    }
}

/// What a stored value is multiplied by to be shown, and its unit.
pub fn param_unit(
    target: editor_core::AnimTarget,
    spec: &editor_core::look::ParamSpec,
) -> (f32, &'static str) {
    use editor_core::{AnimTarget, ParamKey};
    // A clip's place on the canvas is in the canvas's own pixels.
    if let (AnimTarget::Clip, ParamKey::PositionX | ParamKey::PositionY) = (target, spec.key) {
        return (1.0, "");
    }
    let (_, field) = look::presented(target, spec);
    (field.scale, field.suffix)
}

/// What the inspector keeps between frames. None of it is the project's.
#[derive(Debug)]
pub struct InspectorState {
    pub section: Section,
    /// The effect whose rows are open.
    pub effect: Option<Uuid>,
    /// The mask whose rows are open, which the viewer outlines.
    pub mask: Option<Uuid>,
    /// How much of a look the next one given to a clip that has none
    /// brings, `0..=1`. The strength of a look a clip has is read from its
    /// effects (`look::applied_look`) and is not kept here.
    pub strength: f32,
    /// The hue band whose rows are open, the channel whose curve is shown
    /// and the point of it last taken hold of.
    pub band: Option<Uuid>,
    pub channel: Channel,
    pub point: usize,
    /// The clip whose background is to be removed with a clean plate that
    /// has not been chosen yet: a matte is not an effect until it has one.
    plate_wanted: Option<ClipId>,
    /// The key colour a click on the canvas picks next.
    pick: Option<ColourPick>,
    /// The IDs of the effects the Color section makes for a clip when a
    /// value of theirs is first touched, so that one drag makes one effect.
    fresh: Option<(ClipId, Vec<(EffectKind, Uuid)>)>,
    /// How long the next frozen frame is held, in seconds.
    pub hold: f32,
    /// The point of the speed curve last taken hold of, and how it is held.
    graph: speed::GraphState,
    /// The clip whose speed was last asked to be one the project refuses.
    speed_refused: Option<ClipId>,
    /// The frame in which a control that takes the arrows and Delete for
    /// itself last had the keyboard.
    keys: Option<u64>,
    /// An Escape gave up the edit under the pointer or in the focused field:
    /// what that interaction still writes is dropped until it ends.
    cancelled: bool,
    /// The frame in which a field of the inspector was last being typed in.
    typed: Option<u64>,
    /// The box being chosen on the selected clip's picture for a tracker.
    region: Option<TrackRegion>,
    /// The tracking under way, as the application last said.
    tracking: Option<TrackProgress>,
    /// The duck under way, as the application last said.
    ducking: Option<DuckProgress>,
    /// How far a duck asked for here lowers the clip, in dB.
    pub duck_db: f32,
    /// The tracker being renamed, and what has been typed for it.
    rename: Option<(Uuid, String)>,
    /// What is known of the selected clip's camera path, as the application
    /// last said.
    steady: Option<SteadyShown>,
    /// How the steadied layers of the canvas read their pictures.
    readings: Vec<editor_core::ClipOverride>,
    /// A section that names fonts was shown: the families are wanted.
    families_wanted: bool,
    /// What is typed in the search of a list of fonts.
    font_filter: String,
    /// The name a title's look or a graphic is about to be kept under.
    keeping: String,
}

impl Default for InspectorState {
    fn default() -> Self {
        Self {
            section: Section::default(),
            effect: None,
            mask: None,
            strength: 1.0,
            band: None,
            channel: Channel::default(),
            point: 0,
            plate_wanted: None,
            pick: None,
            fresh: None,
            hold: crate::speed::HOLD.as_seconds() as f32,
            graph: speed::GraphState::default(),
            speed_refused: None,
            keys: None,
            cancelled: false,
            typed: None,
            region: None,
            tracking: None,
            ducking: None,
            duck_db: 20.0 * crate::duck::DEPTH.log10(),
            rename: None,
            steady: None,
            readings: Vec::new(),
            families_wanted: false,
            font_filter: String::new(),
            keeping: String::new(),
        }
    }
}

impl InspectorState {
    /// Whether a section that names fonts was shown since this was last
    /// asked: the families are then asked of the title lane.
    pub fn take_families_wanted(&mut self) -> bool {
        std::mem::take(&mut self.families_wanted)
    }

    /// Whether an Escape has a value to give up that the pointer holds. The
    /// pointer's button says so, not what egui is dragging: egui has let go
    /// of its own drag by the time a frame with an Escape in it is drawn.
    pub fn holds(ctx: &egui::Context, draft: &Option<ClipDraft>) -> bool {
        draft.is_some() && ctx.input(|i| i.pointer.any_down())
    }

    /// Whether a field of the inspector had the keyboard when the last frame
    /// was drawn. An Escape is asked this way whose it is: egui takes the
    /// keyboard from the field before the frame with the Escape in it is
    /// drawn, so by then no field is focused.
    pub fn typed(&self, ctx: &egui::Context) -> bool {
        (self.typed).is_some_and(|frame| frame + 1 >= ctx.cumulative_frame_nr())
    }

    /// Gives up the values being dragged or typed: the draft is dropped, and
    /// so is whatever the interaction that made it writes before it ends.
    pub fn cancel(&mut self, draft: &mut Option<ClipDraft>) {
        *draft = None;
        self.cancelled = true;
    }

    /// The key colour the canvas is picking, while the Color section of the
    /// clip it is for is the one shown.
    pub fn colour_pick(&self, selection: &Selection) -> Option<ColourPick> {
        (self.pick)
            .filter(|pick| self.section == Section::Color && selection.single() == Some(pick.clip))
    }

    /// Ends the picking of a key colour: one was picked, or it was given up.
    /// Returns whether one was being picked.
    pub fn end_pick(&mut self) -> bool {
        self.pick.take().is_some()
    }

    /// The region the canvas shows and takes drags for: the one being
    /// chosen, while the Motion section of the clip it is on is shown.
    pub fn track_region(&self, selection: &Selection) -> Option<TrackRegion> {
        (self.region).filter(|region| {
            self.section == Section::Motion && selection.single() == Some(region.clip)
        })
    }

    /// Takes the region as the canvas left it after a drag.
    pub fn set_region(&mut self, region: TrackRegion) {
        if self.region.is_some_and(|held| held.clip == region.clip) {
            self.region = Some(region.held());
        }
    }

    /// Gives up the region being chosen. Returns whether one was.
    pub fn end_region(&mut self) -> bool {
        self.region.take().is_some()
    }

    /// Says what the analysis worker is tracking, or that it is idle.
    pub fn set_tracking(&mut self, tracking: Option<TrackProgress>) {
        self.tracking = tracking;
    }

    /// Says what the duck under way is reading, or that none is.
    pub fn set_ducking(&mut self, ducking: Option<DuckProgress>) {
        self.ducking = ducking;
    }

    /// Says what is known of the selected clip's camera path.
    pub fn set_steady(&mut self, steady: Option<SteadyShown>) {
        self.steady = steady;
    }

    /// Says how the canvas read the picture of each layer it drew steadied:
    /// a clip set to follow a point on one is kept where it is by them.
    pub fn set_readings(&mut self, readings: &[editor_core::ClipOverride]) {
        if self.readings != readings {
            self.readings = readings.to_vec();
        }
    }

    /// Whether the canvas draws the paths of the trackers the selected clip
    /// has to do with: while its Motion section is shown.
    pub fn shows_paths(&self) -> bool {
        self.section == Section::Motion
    }

    /// Whether a control of the inspector that moves with the arrows and
    /// removes with Delete had the keyboard when the last frame was drawn:
    /// those keys are then its own, and not the timeline's. A colour well's
    /// picker is one: its field and strips move with the arrows.
    pub fn owns_keys(&self, ctx: &egui::Context) -> bool {
        let now = ctx.cumulative_frame_nr();
        (self.keys.into_iter())
            .chain(well::had_keys(ctx))
            .any(|frame| frame + 1 >= now)
    }

    /// How long a frame frozen now is held: what the Speed section's field
    /// says.
    pub fn freeze_hold(&self) -> Time {
        Time::from_seconds(f64::from(self.hold))
    }

    /// The mask the viewer outlines: the open one, while its rows are shown.
    pub fn outlined_mask(&self) -> Option<Uuid> {
        self.mask.filter(|_| self.section == Section::Video)
    }
}

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

// ---- Cards and rows -------------------------------------------------------
//
// Every value sits in a card on the control material, as Terminal groups its
// settings: rows 32 tall with a hairline between them, set in 12 from the
// card's leading edge. A card that holds keyed rows reserves their key column
// on every row, so that the fields of all its rows end at one place. The
// card a row is in is kept while the card is laid out, so a row needs only
// its `Ui` to find its hairline and where its controls stop.

/// The room between two cards, and under the last.
const CARD_GAP: f32 = 12.0;
/// A row that holds a segmented control or a select.
const TALL_ROW: f32 = 36.0;
/// Where a card's labels, notes and actions start.
const LEAD: f32 = 12.0;
/// A row of a card without a key column: its controls end 12 in from the
/// card's trailing edge.
const PLAIN: RowFit = RowFit {
    trail: LEAD,
    ..RowFit::INSPECTOR
};
/// The widths of a number field alone and of one of two, and of one of two
/// in a card narrower than [`NARROW_CARD`].
const FIELD: f32 = 72.0;
const PAIRED_FIELD: f32 = 64.0;
const PAIRED_NARROW: f32 = 56.0;
const PAIRED_GAP: f32 = 4.0;
const NARROW_CARD: f32 = 250.0;
/// Where the small buttons of a card's heading end: under the key column's
/// own end, 8 in from the card's edge.
const HEAD_TRAIL: f32 = 8.0;

/// A card being laid out: its edges, how many rows it has had, and whether
/// its rows keep the key column clear.
#[derive(Clone, Copy, Debug)]
struct CardFrame {
    left: f32,
    right: f32,
    rows: usize,
    keyed: bool,
}

/// The cards being laid out, innermost last.
fn cards<R>(ui: &Ui, change: impl FnOnce(&mut Vec<CardFrame>) -> R) -> R {
    ui.data_mut(|data| {
        change(data.get_temp_mut_or_default::<Vec<CardFrame>>(egui::Id::new("inspector-cards")))
    })
}

/// A group of rows on the control material: Terminal's group at the
/// inspector's sizes. Its rows are set off from each other by hairlines.
fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_of(ui, p, false, add)
}

/// As [`card`], for a card that holds keyed rows: every row's controls end
/// where the keyed rows' fields do, clear of the key column.
fn keyed_card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_of(ui, p, true, add)
}

fn card_of<R>(ui: &mut Ui, p: &Palette, keyed: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().top();
    let left = ui.cursor().left();
    let width = ui.available_width();
    cards(ui, |cards| {
        cards.push(CardFrame {
            left,
            right: left + width,
            rows: 0,
            keyed,
        });
    });
    let inner = ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui)
    });
    cards(ui, |cards| cards.pop());
    // A card nothing was put in is not drawn.
    let bottom = inner.response.rect.bottom();
    if bottom > top + 0.5 {
        let rect = Rect::from_min_max(pos2(left, top), pos2(left + width, bottom));
        ui.painter().set(
            background,
            egui::Shape::rect_filled(rect, metrics::PANE_RADIUS, p.control),
        );
        ui.add_space(CARD_GAP);
    }
    inner.inner
}

/// Marks the start of a row of the card being laid out: a hairline is drawn
/// over it, set in 12 from the card's leading edge, when a row came before.
fn rule(ui: &Ui, p: &Palette) {
    let y = ui.cursor().top();
    let line = cards(ui, |cards| {
        let card = cards.last_mut()?;
        card.rows += 1;
        (card.rows > 1).then_some((card.left, card.right))
    });
    if let Some((left, right)) = line {
        ui.painter().hline(left + LEAD..=right, y, p.hairline());
    }
}

fn in_card(ui: &Ui) -> bool {
    cards(ui, |cards| !cards.is_empty())
}

/// Where a plain row's controls stop: clear of the key column in a card that
/// holds keyed rows.
fn fit(ui: &Ui) -> RowFit {
    if cards(ui, |cards| cards.last().is_some_and(|card| card.keyed)) {
        RowFit::INSPECTOR_BESIDE_KEYS
    } else {
        PLAIN
    }
}

/// The width of one of two fields in a row of the card being laid out.
fn paired_width(ui: &Ui) -> f32 {
    let width = cards(ui, |cards| cards.last().map(|card| card.right - card.left));
    if width.unwrap_or_else(|| ui.available_width()) < NARROW_CARD {
        PAIRED_NARROW
    } else {
        PAIRED_FIELD
    }
}

/// A label at the leading edge of a row and the controls `add` makes at its
/// trailing edge.
fn row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    rule(ui, p);
    let fit = fit(ui);
    kit::row(ui, p, label, fit, add)
}

/// Controls `add` makes in `width` at the trailing edge of a row, laid out
/// in the order they are read. A row adds its controls from the trailing
/// edge, and Tab would walk two of them right to left (§4.3.3, §7).
fn in_reading_order<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let height = ui.available_height();
    ui.allocate_ui_with_layout(vec2(width, height), layout, add)
        .inner
}

/// As [`row`], with a label that is cut with an ellipsis rather than pushed
/// under its controls when the card is narrow.
fn fitted_row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    rule(ui, p);
    let fit = fit(ui);
    kit::fitted_row(ui, p, label, fit, add)
}

/// As [`fitted_row`], 36 tall: a row that holds a select.
fn tall_row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    rule(ui, p);
    let fit = RowFit {
        height: TALL_ROW,
        ..fit(ui)
    };
    kit::fitted_row(ui, p, label, fit, add)
}

/// A row whose value can be keyframed: its fields, then the key column.
fn keyed_row(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    what: &str,
    state: KeyState,
    add: impl FnOnce(&mut Ui),
    lead: impl FnOnce(&mut Ui),
) -> KeyRow {
    rule(ui, p);
    kit::keyed_row(ui, p, label, what, state, RowFit::INSPECTOR, add, lead)
}

/// A row `height` tall that the caller fills across the card, set in by the
/// card's lead on both sides.
fn wide_row<R>(ui: &mut Ui, p: &Palette, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    rule(ui, p);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let salt = ui.next_auto_id();
    controls::place(
        ui,
        rect.shrink2(vec2(LEAD, 0.0)),
        egui::Layout::left_to_right(egui::Align::Center),
        salt,
        add,
    )
}

/// A row of a segmented control across the card.
fn choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    wide_row(ui, p, TALL_ROW, |ui| {
        let width = ui.available_width();
        widgets::segmented(ui, p, value, options, width)
    })
}

/// A row of buttons at the card's leading edge, wrapped where the card is
/// narrow. Outside a card, the same row under the cards, set in 4 as notes
/// are there, so that a button's focus ring is clear of the pane's edge
/// (§7.4).
fn button_row<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    rule(ui, p);
    let carded = in_card(ui);
    let lead = if carded { LEAD as i8 } else { 4 };
    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: lead,
            right: lead,
            top: if carded { 5 } else { 0 },
            bottom: if carded { 5 } else { 0 },
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                add(ui)
            })
            .inner
        })
        .inner;
    if !carded {
        ui.add_space(CARD_GAP);
    }
    inner
}

/// A labelled value that is read, not edited.
fn fact(ui: &mut Ui, p: &Palette, label: &str, value: impl Into<String>) {
    fact_in(ui, p, label, value.into(), theme::regular(12.5));
}

/// As [`fact`], for a time: set in the timecode's face, as the header's is.
fn time_fact(ui: &mut Ui, p: &Palette, label: &str, value: impl Into<String>) {
    fact_in(ui, p, label, value.into(), theme::mono(11.0));
}

fn fact_in(ui: &mut Ui, p: &Palette, label: &str, value: String, font: egui::FontId) {
    fitted_row(ui, p, label, |ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(value).font(font).color(p.secondary)).truncate(),
        );
    });
}

/// Supporting text beneath the rows it explains: inside a card under a
/// hairline, as Terminal's group notes are; outside one, under the cards.
fn note(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    rule(ui, p);
    let carded = in_card(ui);
    let response = egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: if carded { LEAD as i8 } else { 4 },
            right: if carded { LEAD as i8 } else { 4 },
            top: if carded { 10 } else { 0 },
            bottom: if carded { 10 } else { 0 },
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .font(theme::regular(11.5))
                        .color(p.secondary),
                )
                .wrap(),
            )
        })
        .inner;
    if !carded {
        ui.add_space(CARD_GAP);
    }
    response
}

/// A small label over the cards of one part of a section.
fn section(ui: &mut Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    controls::place(
        ui,
        Rect::from_min_max(pos2(rect.left() + 4.0, rect.top()), rect.max),
        egui::Layout::left_to_right(egui::Align::Center),
        ("section", text),
        |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .font(theme::medium(11.5))
                        .color(p.secondary),
                )
                .truncate(),
            );
        },
    );
}

/// The heading row of a card: the name of what the card is (an effect, a
/// mask, a tracker, a group of the Color section) and the small buttons that
/// act on it, which stay in view. With `open`, the name is a disclosure that
/// shows and hides the card's rows, named by the name. Hands back the
/// disclosure, when there is one. `trailing` is how many small buttons the
/// heading ends with and what lays them out, in reading order, so that Tab
/// takes the name and then them from left to right.
fn card_head(
    ui: &mut Ui,
    p: &Palette,
    name: &str,
    open: Option<bool>,
    dim: bool,
    (count, trailing): (usize, impl FnOnce(&mut Ui)),
) -> Option<Response> {
    rule(ui, p);
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), metrics::INSPECTOR_ROW),
        Sense::hover(),
    );
    let surface = ui.painter().add(egui::Shape::Noop);
    let start = rect.right() - HEAD_TRAIL - count as f32 * metrics::SMALL_BUTTON;
    let end = if count > 0 { start - 4.0 } else { start };
    // The disclosure is made first, so that Tab reaches it and then the
    // buttons. It stops short of them: a button that cannot be used senses
    // nothing, and a click on it must not fall through to the name.
    let header = open.map(|open| {
        let name_area = Rect::from_min_max(rect.min, pos2(end, rect.bottom()));
        let response = ui.interact(name_area, ui.id().with(("card-head", name)), Sense::click());
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::CollapsingHeader, ui.is_enabled(), name);
            info.selected = Some(open);
            info
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_expanded(open);
        });
        response
    });
    controls::place(
        ui,
        Rect::from_min_max(pos2(start, rect.top()), rect.max),
        egui::Layout::left_to_right(egui::Align::Center),
        ("card-head-actions", name),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            trailing(ui);
        },
    );
    let ink = if dim || !ui.is_enabled() {
        p.muted
    } else {
        p.fg
    };
    // Neptune Video: Terminal's disclosure puts its chevron at +20 and its
    // name at +36 from a 14-point lead. A card's lead is 12, so the chevron
    // sits at +18, its glyph's edge on the rows' labels, and the name keeps
    // Terminal's 16 points from it.
    let text_left = if open.is_some() {
        rect.left() + 34.0
    } else {
        rect.left() + LEAD
    };
    match (&header, open) {
        (Some(header), Some(open)) => {
            if ui.is_rect_visible(rect) {
                let area = Rect::from_min_max(rect.min, pos2(end, rect.bottom())).shrink(4.0);
                let painter = ui.painter();
                let hover = kit::hover_amount(ui, header);
                let mut shapes = Vec::new();
                if ui.is_enabled() && header.is_pointer_button_down_on() {
                    shapes.push(egui::Shape::rect_filled(area, 6, p.pressed));
                } else if hover > 0.0 {
                    shapes.push(egui::Shape::rect_filled(
                        area,
                        6,
                        p.hover.gamma_multiply(hover),
                    ));
                }
                painter.set(surface, egui::Shape::Vec(shapes));
                if header.has_focus() {
                    controls::focus_ring(painter, area, 6, *p);
                }
                icons::paint(
                    painter,
                    Rect::from_center_size(
                        pos2(rect.left() + 18.0, rect.center().y),
                        Vec2::splat(14.0),
                    ),
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    p.secondary,
                );
                let galley = controls::elided(
                    painter,
                    name,
                    theme::medium(12.5),
                    ink,
                    end - 4.0 - text_left,
                );
                controls::galley_at(painter, pos2(text_left, rect.center().y), galley);
            }
        }
        _ => {
            controls::place(
                ui,
                Rect::from_min_max(pos2(text_left, rect.top()), pos2(end, rect.bottom())),
                egui::Layout::left_to_right(egui::Align::Center),
                ("card-head-name", name),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(name)
                                .font(theme::medium(12.5))
                                .color(ink),
                        )
                        .truncate(),
                    );
                },
            );
        }
    }
    header.map(|header| {
        if ui.is_enabled() {
            header.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            header
        }
    })
}

/// A small icon button of a card's heading, which may be out of reach.
fn head_button(ui: &mut Ui, p: &Palette, icon: Icon, enabled: bool, label: &str) -> bool {
    ui.add_enabled_ui(enabled, |ui| kit::small_icon_button(ui, p, icon, label))
        .inner
        .clicked()
}

/// A compact button for the inside of a card, 26 tall, led by an icon where
/// it has one, and read out by what it shows. `controls::button_sized` is
/// the same button without an icon.
fn button(ui: &mut Ui, p: &Palette, icon: Option<Icon>, text: &str, kind: Kind) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), theme::medium(12.5), Color32::PLACEHOLDER);
    // The icon side takes two points less padding than the text side.
    let lead = if icon.is_some() { 10.0 } else { 12.0 };
    let icon_width = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let content = icon_width + galley.size().x;
    let width = (lead + content + 12.0).max(56.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, metrics::INSPECTOR_CONTROL), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let (fill, ink) = match kind {
        Kind::Primary => (p.accent, p.on_accent),
        Kind::Destructive => (p.red, p.on_red()),
        Kind::Plain => (p.control, p.fg),
        Kind::Quiet => (Color32::TRANSPARENT, p.secondary),
    };
    let solid = matches!(kind, Kind::Primary | Kind::Destructive);
    let radius = metrics::CONTROL_RADIUS;
    let hover = kit::hover_amount(ui, &response);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    if enabled && response.is_pointer_button_down_on() {
        let down = if solid {
            Color32::from_black_alpha(46)
        } else {
            p.pressed
        };
        painter.rect_filled(rect, radius, down);
    } else if hover > 0.0 {
        let lift = if solid {
            Color32::from_white_alpha(26)
        } else {
            p.hover
        };
        painter.rect_filled(rect, radius, lift.gamma_multiply(hover));
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, radius, *p);
    }
    let ink = if enabled { ink } else { p.muted };
    let mut x = rect.center().x - content * 0.5;
    if let Some(icon) = icon {
        let glyph = Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(14.0));
        icons::paint(painter, glyph, icon, ink);
        x += icon_width;
    }
    painter.galley(
        pos2(x, rect.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A number field of a card, 26 tall and `width` wide, named `label`.
#[expect(clippy::too_many_arguments, reason = "a field is all of these")]
fn number(
    ui: &mut Ui,
    p: &Palette,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    suffix: &str,
    decimals: usize,
    width: f32,
    label: &str,
) -> Response {
    number_with(
        ui,
        p,
        value,
        &range,
        speed,
        (None, suffix),
        decimals,
        width,
        label,
    )
}

/// As [`number`], with the axis before the value: "X", "Y", "W", "H".
#[expect(clippy::too_many_arguments, reason = "a field is all of these")]
fn number_with(
    ui: &mut Ui,
    p: &Palette,
    value: &mut f32,
    range: &std::ops::RangeInclusive<f32>,
    speed: f64,
    (prefix, suffix): (Option<&str>, &str),
    decimals: usize,
    width: f32,
    label: &str,
) -> Response {
    let number = kit::Number {
        range,
        speed,
        prefix,
        suffix,
        decimals,
    };
    kit::number_field(
        ui,
        p,
        value,
        number,
        vec2(width, metrics::INSPECTOR_CONTROL),
        Some(label),
    )
}

/// A select over named choices, 26 tall, as a row of a card holds it.
fn select_list<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    salt: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> Response {
    let id = ui.make_persistent_id(salt);
    let size = vec2(width, metrics::INSPECTOR_CONTROL);
    kit::select_list(ui, p, id, label, value, options, size)
}

/// A select whose list the caller lays out: the field, and the list it
/// opens, which `rows` fills with menu rows. Closed by a choice. A list of
/// no fixed length scrolls once it is taller than 260 points, so its last
/// rows are in reach in any window.
fn select_menu<R>(
    ui: &mut Ui,
    p: &Palette,
    named: (impl std::hash::Hash + std::fmt::Debug, &str),
    value: &str,
    width: f32,
    rows: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<R>) {
    select_popup(ui, p, named, value, width, true, rows)
}

/// As [`select_menu`]; unless `scrolled`, the list is shown whole, as a
/// list of a fixed few choices is, or `rows` scrolls what it has to itself,
/// as a list under a search field that stays in view does.
fn select_popup<R>(
    ui: &mut Ui,
    p: &Palette,
    (salt, label): (impl std::hash::Hash + std::fmt::Debug, &str),
    value: &str,
    width: f32,
    scrolled: bool,
    rows: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<R>) {
    let id = ui.make_persistent_id(salt);
    let size = vec2(width, metrics::INSPECTOR_CONTROL);
    let response = kit::select(ui, p, id, value, label, size);
    let list = egui::Popup::menu(&response).show(|ui| {
        controls::menu_layout(ui, width.max(240.0));
        if scrolled {
            egui::ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .show(ui, rows)
                .inner
        } else {
            rows(ui)
        }
    });
    (response, list.map(|list| list.inner))
}

/// The height past which a select's list scrolls.
const LIST_HEIGHT: f32 = 260.0;

/// A one-line text field of a card, 26 tall: Terminal's text field at the
/// inspector's size.
fn text_field(
    ui: &mut Ui,
    p: &Palette,
    id: egui::Id,
    text: &mut String,
    label: &str,
    width: f32,
) -> Response {
    let (rect, surface) =
        ui.allocate_exact_size(vec2(width, metrics::INSPECTOR_CONTROL), Sense::hover());
    controls::field_surface(ui, *p, rect, id, (true, 7));
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let response = controls::place(
        ui,
        inner,
        egui::Layout::left_to_right(egui::Align::Center),
        ("text-field", id),
        |ui| controls::bare_text_edit(ui, id, text, "", 12.5, inner.width()),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, label));
    // The whole field takes the pointer, not only the line of text in it:
    // a click on its padding puts the caret in it too. The surface is no
    // stop of its own for Tab.
    if ui.is_enabled() && surface.contains_pointer() && !response.contains_pointer() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        if ui.input(|i| i.pointer.primary_clicked()) {
            let edit = controls::viewport_id(ui.ctx(), id);
            ui.memory_mut(|memory| memory.request_focus(edit));
        }
    }
    response
}

// ---- The object header ----------------------------------------------------

/// The panel height under which the header is one line, so that the rows
/// under it keep most of the panel.
const COMPACT_HEAD: f32 = 300.0;
/// The header's height, full and on one line, and the room under it.
const HEAD: f32 = 44.0;
const HEAD_COMPACT: f32 = 28.0;
/// The section switch, and the room above and under it.
const SWITCH: f32 = 28.0;
/// The room the statement that nothing is selected takes over the
/// project's cards, and in a short panel, where it is said without its
/// glyph so that the cards under it keep the room.
const NOTHING_SELECTED: f32 = 112.0;
const NOTHING_SELECTED_COMPACT: f32 = 48.0;

/// What the header's thumbnail shows.
#[derive(Clone, Copy)]
enum Thumb<'a> {
    /// A picture's poster, with its kind's glyph until the poster is read.
    Picture(Option<&'a egui::TextureHandle>, Icon, Ink),
    /// A glyph for what has no picture: a title, a sound, the project.
    Glyph(Icon, Ink),
}

/// The ink of a thumbnail's glyph, said by what it is rather than as a
/// colour: on the 48-point tile it is the stage palette's, as everything
/// on the stage is (§3.2), and on the one-line header the panel's own.
#[derive(Clone, Copy)]
enum Ink {
    Secondary,
    Tone(theme::Tone),
    Wave,
}

impl Ink {
    fn on(self, p: &Palette) -> Color32 {
        match self {
            Self::Secondary => p.secondary,
            Self::Tone(tone) => p.tone(tone),
            Self::Wave => p.wave(),
        }
    }
}

/// The header of what the inspector shows, 44 tall in `rect`: a thumbnail on
/// the stage's dark, the name, and a line of detail with the time set in the
/// timecode's face. One line of 28 when `compact`.
fn object_header(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    thumb: Thumb,
    name: &str,
    detail: (&str, &str),
    compact: bool,
) {
    let (words, time) = detail;
    let format = |font, color| egui::TextFormat {
        font_id: font,
        color,
        ..Default::default()
    };
    if compact {
        let glyph = match thumb {
            Thumb::Picture(_, icon, ink) | Thumb::Glyph(icon, ink) => (icon, ink),
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), Vec2::splat(14.0)),
            glyph.0,
            glyph.1.on(p),
        );
        // The detail follows the name only where it fits whole: a time cut
        // short says nothing. Without the time, the words go on their own
        // where they fit.
        let left = rect.left() + 26.0;
        let width = |text: &str, font: egui::FontId| {
            (ui.painter())
                .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
                .size()
                .x
        };
        let room = rect.right() - left - width(name, theme::medium(12.5)) - 8.0;
        let bare = words.trim_end_matches(" · ");
        // What is read out is the whole line, however much of it is shown.
        let said = match (bare.is_empty(), time.is_empty()) {
            (true, true) => name.to_owned(),
            (false, true) => format!("{name} {bare}"),
            _ => format!("{name} {words}{time}"),
        };
        let (words, time) =
            if width(words, theme::regular(11.5)) + width(time, theme::mono(11.0)) <= room {
                (words, time)
            } else if width(bare, theme::regular(11.5)) <= room {
                (bare, "")
            } else {
                ("", "")
            };
        let mut job = egui::text::LayoutJob::default();
        job.append(name, 0.0, format(theme::medium(12.5), p.fg));
        if !words.is_empty() || !time.is_empty() {
            // A space of its own, so that the name and the detail are read
            // out as two runs of words; the gap makes up the 8 points.
            job.append(" ", 5.0, format(theme::regular(11.5), p.secondary));
        }
        job.append(words, 0.0, format(theme::regular(11.5), p.secondary));
        job.append(time, 0.0, format(theme::mono(11.0), p.secondary));
        controls::place(
            ui,
            Rect::from_min_max(pos2(left, rect.top()), rect.max),
            egui::Layout::left_to_right(egui::Align::Center),
            "object-header",
            |ui| {
                ui.add(egui::Label::new(job).truncate())
                    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &said));
            },
        );
        return;
    }
    let tile = Rect::from_min_size(
        pos2(rect.left() + 6.0, rect.center().y - 13.5),
        vec2(48.0, 27.0),
    );
    let painter = ui.painter();
    painter.rect_filled(tile, 5, p.stage());
    match thumb {
        Thumb::Picture(Some(texture), _, _) => {
            // The poster covers the tile; what is past its shape is cut.
            let size = texture.size_vec2();
            let scale = (tile.width() / size.x.max(1.0)).max(tile.height() / size.y.max(1.0));
            let shown = tile.size() / (size * scale);
            let uv = Rect::from_center_size(pos2(0.5, 0.5), shown);
            painter.add(
                RectShape::filled(tile, CornerRadius::same(5), Color32::WHITE)
                    .with_texture(texture.id(), uv),
            );
        }
        Thumb::Picture(None, icon, ink) | Thumb::Glyph(icon, ink) => {
            // The tile is the stage's dark in either appearance. None of
            // these inks follows the accent.
            let stage = Palette::stage_palette(theme::Accent::default());
            icons::paint(
                painter,
                Rect::from_center_size(tile.center(), Vec2::splat(14.0)),
                icon,
                ink.on(&stage),
            );
        }
    }
    painter.rect_stroke(
        tile,
        5,
        Stroke::new(1.0, p.canvas_edge()),
        StrokeKind::Inside,
    );
    let text = Rect::from_min_max(pos2(rect.left() + 62.0, rect.top()), rect.max);
    let line = |top: f32, bottom: f32| {
        Rect::from_min_max(pos2(text.left(), top), pos2(text.right(), bottom))
    };
    controls::place(
        ui,
        line(rect.top() + 5.0, rect.top() + 23.0),
        egui::Layout::left_to_right(egui::Align::Center),
        "object-name",
        |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(name)
                        .font(theme::medium(13.0))
                        .color(p.fg),
                )
                .truncate(),
            )
        },
    );
    if words.is_empty() && time.is_empty() {
        return;
    }
    let mut job = egui::text::LayoutJob::default();
    job.append(words, 0.0, format(theme::regular(11.5), p.secondary));
    job.append(time, 0.0, format(theme::mono(11.0), p.secondary));
    controls::place(
        ui,
        line(rect.bottom() - 21.0, rect.bottom() - 5.0),
        egui::Layout::left_to_right(egui::Align::Center),
        "object-detail",
        |ui| ui.add(egui::Label::new(job).truncate()),
    );
}

/// The glyph a section is shown by when the switch is too narrow for names.
fn section_icon(section: Section) -> Icon {
    match section {
        Section::Video => Icon::Film,
        Section::Look => Icon::ColorSwatch,
        Section::Color => Icon::Palette,
        Section::Speed => Icon::Curve,
        Section::Motion => Icon::Fit,
        Section::Audio => Icon::Volume,
        Section::Text => Icon::Text,
        Section::Graphic => Icon::Layers,
        Section::Animate => Icon::Transition,
    }
}

/// The sections a clip is offered, as Terminal's segmented control across
/// `rect`: names in Medium 12, then 11.5, then glyphs named by their
/// tooltips where the names do not fit. Left and Right move the choice.
/// Hands back whether another section was chosen.
fn section_switch(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    offered: &[Section],
    section: &mut Section,
) -> bool {
    let id = ui.id().with("inspector-sections");
    let count = offered.len().max(1);
    let segment = (rect.width() - 4.0) / count as f32;
    let cells: Vec<egui::Id> = (offered.iter())
        .map(|section| id.with(section.label()))
        .collect();
    let mut changed = false;
    let at = |section: Section| offered.iter().position(|offered| *offered == section);
    if let Some(to) = super::arrow_choice(ui.ctx(), &cells, at(*section).unwrap_or(0)) {
        *section = offered[to];
        changed = true;
    }
    let selected = at(*section).unwrap_or(0);
    let painter = ui.painter().clone();
    let room = |size: f32| {
        offered.iter().all(|section| {
            let galley = painter.layout_no_wrap(
                section.label().to_owned(),
                theme::medium(size),
                Color32::PLACEHOLDER,
            );
            galley.size().x + controls::SEGMENT_SNUG * 2.0 <= segment
        })
    };
    let font = if room(12.0) {
        Some(12.0)
    } else if room(11.5) {
        Some(11.5)
    } else {
        None
    };
    painter.rect_filled(rect, 8, p.control);
    let slide = if kit::reduced_motion(ui.ctx()) {
        0.0
    } else {
        theme::motion::SEGMENTED
    };
    let thumb_x = ui
        .ctx()
        .animate_value_with_time(id.with("thumb"), selected as f32, slide);
    let thumb = Rect::from_min_size(
        pos2(rect.left() + 2.0 + thumb_x * segment, rect.top() + 2.0),
        vec2(segment, rect.height() - 4.0),
    );
    painter.rect_filled(
        thumb.translate(vec2(0.0, 1.0)),
        6,
        Color32::from_black_alpha(if p.dark { 50 } else { 22 }),
    );
    painter.rect_filled(
        thumb,
        6,
        if p.dark {
            theme::mix(p.elevated, Color32::WHITE, 0.16)
        } else {
            Color32::WHITE
        },
    );
    for (index, option) in offered.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(rect.left() + 2.0 + index as f32 * segment, rect.top()),
            vec2(segment, rect.height()),
        );
        let response = ui.interact(cell, cells[index], Sense::click());
        let active = index == selected;
        let label = option.label();
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), active, label)
        });
        if response.clicked() && !active {
            *section = *option;
            changed = true;
        }
        if response.has_focus() {
            controls::focus_ring(&painter, cell.shrink(2.0), 6, *p);
        }
        let ink = if active || response.hovered() {
            p.fg
        } else {
            p.secondary
        };
        let response = match font {
            Some(size) => {
                painter.text(
                    cell.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    theme::medium(size),
                    ink,
                );
                response
            }
            None => {
                icons::paint(
                    &painter,
                    Rect::from_center_size(cell.center(), Vec2::splat(14.0)),
                    section_icon(*option),
                    ink,
                );
                response.on_hover_text(label)
            }
        };
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    changed
}

#[expect(clippy::too_many_arguments, reason = "one call site in the frame")]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut InspectorState,
    selection: &Selection,
    draft: &mut Option<ClipDraft>,
    project_draft: &mut ProjectDraft,
    input: &InspectorInput,
    actions: &mut Vec<Action>,
) {
    // No card is open between frames, whatever a frame given up left.
    cards(ui, Vec::clear);
    panes(
        ui,
        rect,
        state,
        selection,
        draft,
        project_draft,
        input,
        actions,
    );
    let typing = ui.ctx().text_edit_focused();
    if typing {
        state.typed = Some(ui.ctx().cumulative_frame_nr());
    }
    // A cancelled interaction is over once nothing is held or typed in. The
    // frame that gave it up is drawn first, so nothing it writes is kept.
    if state.cancelled && !ui.input(|i| i.pointer.any_down()) && !typing {
        state.cancelled = false;
    }
}

#[expect(clippy::too_many_arguments, reason = "one call site in the frame")]
fn panes(
    ui: &mut Ui,
    rect: Rect,
    state: &mut InspectorState,
    selection: &Selection,
    draft: &mut Option<ClipDraft>,
    project_draft: &mut ProjectDraft,
    input: &InspectorInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let project = input.project;
    // A short panel gives the rows the room: the header is one line.
    let compact = rect.height() < COMPACT_HEAD;
    let head = Rect::from_min_size(
        rect.min,
        vec2(rect.width(), if compact { HEAD_COMPACT } else { HEAD }),
    );
    let below =
        |bottom: f32| Rect::from_min_max(pos2(rect.left(), bottom.min(rect.bottom())), rect.max);
    // A clip's header and its sections stay where they are while its rows
    // scroll under them, and each section keeps its own place.
    if let Some((track, clip)) = selection.single().and_then(|id| project.clip(id)) {
        if let Some(edge) =
            (input.transition).filter(|edge| super::transitions::of(clip, *edge).is_some())
        {
            transition::head(ui, input, head, clip, edge, compact);
            scrolled(
                ui,
                p,
                below(head.bottom() + 6.0),
                ("inspector-transition", edge),
                |ui| {
                    transition::pane(ui, input, track, clip, edge, state, draft, actions);
                },
            );
            return;
        }
        let (section, bottom) = clip::head(ui, input, head, track, clip, state, compact);
        scrolled(
            ui,
            p,
            below(bottom + 6.0),
            ("inspector-clip", section),
            |ui| {
                clip::rows(
                    ui,
                    input,
                    track,
                    clip,
                    section,
                    state,
                    draft,
                    project_draft,
                    actions,
                );
            },
        );
        return;
    }
    if selection.clips.len() > 1 {
        let clips = selection.clips.len();
        object_header(
            ui,
            p,
            head,
            Thumb::Glyph(Icon::Layers, Ink::Secondary),
            &format!("{clips} clips"),
            ("Selected together", ""),
            compact,
        );
        scrolled(
            ui,
            p,
            below(head.bottom() + 6.0),
            "inspector-several",
            |ui| {
                several_pane(ui, p, project, selection, actions);
            },
        );
    } else if let Some(asset) = selection.asset.and_then(|id| project.assets.get(&id)) {
        scrolled(ui, p, rect, "inspector-asset", |ui| {
            asset_pane(ui, p, asset, input.textures, actions);
        });
    } else {
        // Nothing selected says so (§5.10), and over the project's own
        // settings, which are edited nowhere else.
        let clips: usize = project.tracks.iter().map(|track| track.clips.len()).sum();
        let detail = match clips {
            1 => "1 clip · ".to_owned(),
            n => format!("{n} clips · "),
        };
        object_header(
            ui,
            p,
            head,
            Thumb::Glyph(Icon::Film, Ink::Tone(theme::Tone::Video)),
            &project.name,
            (&detail, &timefmt::short(project.duration())),
            compact,
        );
        scrolled(ui, p, below(head.bottom() + 6.0), "inspector", |ui| {
            let (title, hint) = ("Nothing selected", "Select a clip to see its properties.");
            let room = if compact {
                NOTHING_SELECTED_COMPACT
            } else {
                NOTHING_SELECTED
            };
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), room), Sense::hover());
            if compact {
                kit::small_empty_state(ui, p, rect, Icon::Pointer, title, hint);
            } else {
                kit::empty_state(ui, p, rect, Icon::Pointer, title, hint, None);
            }
            project_pane(ui, p, project, project_draft);
        });
    }
}

/// A pane's rows in `rect`, scrolled when there are more than fit, with a
/// fade at the foot while more are under it. A row the keyboard moves to is
/// scrolled into view.
fn scrolled(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut Ui),
) {
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    // The rows are cut off above and under the pane, but not at its sides:
    // there they are cut 3 out, in the panel's gutter, so that the ring of
    // a control on the cards' edge is whole (§7.4).
    body.set_clip_rect(rect.expand2(vec2(3.0, 0.0)).intersect(ui.clip_rect()));
    // Its bar shows without the pointer over it, as Terminal's does: a fade
    // in the gap between two cards is chrome on chrome, and the bar is what
    // says that the rows go on.
    let shown = &mut body.spacing_mut().scroll;
    shown.dormant_handle_opacity = shown.active_handle_opacity;
    // Where the floating scroll bar starts, which a point's target stops
    // short of: the bar is laid over the rows and takes what is under it.
    let scroll = ui.spacing().scroll;
    let bar = rect.right() - scroll.bar_outer_margin - scroll.bar_width;
    ui.data_mut(|data| data.insert_temp(egui::Id::new("inspector-bar"), bar));
    let output = egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show(&mut body, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
            // The control the keyboard has, when it has just moved there
            // without the pointer and is one of these rows.
            let focused = ui.memory(|memory| memory.focused());
            let seen = egui::Id::new("inspector-focus-seen");
            let before: Option<egui::Id> = ui.data(|data| data.get_temp(seen));
            if focused != before {
                ui.data_mut(|data| match focused {
                    Some(id) => {
                        data.insert_temp(seen, id);
                    }
                    None => data.remove::<egui::Id>(seen),
                });
                let by_pointer = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
                // Reading a response is egui's test of a click elsewhere: on
                // a click it would take the keyboard from a control another
                // panel has just focused.
                if !by_pointer
                    && let Some(row) = focused.and_then(|id| ui.ctx().read_response(id))
                    && ui.min_rect().contains(row.rect.center())
                {
                    ui.scroll_to_rect(row.rect.expand2(vec2(0.0, 10.0)), None);
                }
            }
        });
    // A fade on each side that rows run on past.
    kit::scroll_fades(ui, &output, p.chrome);
}

/// Where the floating scroll bar of the pane being laid out starts. A
/// target that would run under it, as a curve's last point's would, is cut
/// short there, so that a press beside the point takes the point.
fn bar_start(ui: &Ui) -> f32 {
    (ui.data(|data| data.get_temp(egui::Id::new("inspector-bar")))).unwrap_or(f32::INFINITY)
}

/// A point's target, cut short of the scroll bar.
fn clear_of_bar(ui: &Ui, target: Rect) -> Rect {
    let mut target = target;
    target.max.x = target.max.x.min(bar_start(ui)).max(target.min.x);
    target
}

fn several_pane(
    ui: &mut Ui,
    p: &Palette,
    project: &Project,
    selection: &Selection,
    actions: &mut Vec<Action>,
) {
    let ids = selection.clips.clone();
    let chosen: Vec<(&editor_core::Track, &Clip)> =
        ids.iter().filter_map(|id| project.clip(*id)).collect();
    let any = |test: fn(&Clip) -> bool| chosen.iter().any(|(_, clip)| test(clip));
    let rate = project.settings.frame_rate;
    // A value the clips do not share is shown as a dash.
    let mixed = "—";
    section(ui, p, "Selection");
    card(ui, p, |ui| {
        let tracks = chosen.iter().map(|(track, _)| track.id);
        let track = match chosen.first() {
            Some((first, _)) if tracks.clone().all(|id| id == first.id) => first.name.clone(),
            _ => mixed.to_owned(),
        };
        fact(ui, p, "Track", track);
        let start = chosen.iter().map(|(_, clip)| clip.start).min();
        let end = chosen.iter().map(|(_, clip)| clip.end()).max();
        if let (Some(start), Some(end)) = (start, end) {
            time_fact(ui, p, "Start", timefmt::timecode(start, rate));
            time_fact(ui, p, "End", timefmt::timecode(end, rate));
        }
        let duration = match chosen.first() {
            Some((_, first))
                if chosen
                    .iter()
                    .all(|(_, clip)| clip.duration == first.duration) =>
            {
                timefmt::timecode(first.duration, rate)
            }
            _ => mixed.to_owned(),
        };
        time_fact(ui, p, "Duration", duration);
    });
    section(ui, p, "Arrange");
    card(ui, p, |ui| {
        button_row(ui, p, |ui| {
            if button(ui, p, Some(Icon::Layers), "Group", Kind::Plain).clicked() {
                actions.push(Action::Edit(Command::GroupClips {
                    clip_ids: ids.clone(),
                }));
            }
            if button(ui, p, Some(Icon::Link), "Link", Kind::Plain).clicked() {
                actions.push(Action::Edit(Command::LinkClips {
                    clip_ids: ids.clone(),
                }));
            }
            if any(|clip| clip.group_id.is_some())
                && button(ui, p, None, "Ungroup", Kind::Plain).clicked()
            {
                actions.push(Action::Edit(Command::UngroupClips {
                    clip_ids: ids.clone(),
                }));
            }
            if any(|clip| clip.linked_id.is_some())
                && button(ui, p, None, "Unlink", Kind::Plain).clicked()
            {
                actions.push(Action::Edit(Command::UnlinkClips {
                    clip_ids: ids.clone(),
                }));
            }
        });
        button_row(ui, p, |ui| {
            if button(ui, p, Some(Icon::Trash), "Delete", Kind::Plain).clicked() {
                actions.push(Action::Delete { ripple: false });
            }
            if button(ui, p, None, "Ripple delete", Kind::Plain).clicked() {
                actions.push(Action::Delete { ripple: true });
            }
        });
    });
}

fn asset_pane(
    ui: &mut Ui,
    p: &Palette,
    asset: &Asset,
    textures: &Textures,
    actions: &mut Vec<Action>,
) {
    let tone = p.tone(tone_of_media(asset.kind));
    // The poster, at the panel's width.
    let width = ui.available_width();
    let (picture, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::hover());
    let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
    ui.painter().rect_filled(picture, radius, p.stage());
    match textures.poster(asset.id) {
        Some(texture) => {
            // Fitted inside the frame, with the frame's corners where the
            // poster reaches them.
            let size = texture.size_vec2();
            let scale = (picture.width() / size.x).min(picture.height() / size.y);
            let fitted = Rect::from_center_size(picture.center(), size * scale);
            let reach = |near: f32, edge: f32| (near - edge).abs() < 1.0;
            let round = |top: bool, left: bool| {
                let vertical = if top {
                    reach(fitted.top(), picture.top())
                } else {
                    reach(fitted.bottom(), picture.bottom())
                };
                let horizontal = if left {
                    reach(fitted.left(), picture.left())
                } else {
                    reach(fitted.right(), picture.right())
                };
                if vertical && horizontal {
                    metrics::CONTROL_RADIUS
                } else {
                    0
                }
            };
            let corners = CornerRadius {
                nw: round(true, true),
                ne: round(true, false),
                sw: round(false, true),
                se: round(false, false),
            };
            ui.painter().add(
                RectShape::filled(fitted, corners, Color32::WHITE).with_texture(
                    texture.id(),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                ),
            );
        }
        None => icons::paint(
            ui.painter(),
            Rect::from_center_size(picture.center(), Vec2::splat(30.0)),
            icon_of_media(asset.kind),
            tone,
        ),
    }
    ui.painter().rect_stroke(
        picture,
        radius,
        Stroke::new(1.0, p.canvas_edge()),
        StrokeKind::Inside,
    );
    ui.add_space(12.0);
    let kind = match asset.kind {
        MediaKind::Video => "Video",
        MediaKind::Audio => "Audio",
        MediaKind::Image => "Image",
        MediaKind::Gif => "GIF",
        MediaKind::Lut => "LUT",
        MediaKind::Font => "Font",
    };
    // A still has no length of its own, and a lookup table no time,
    // picture or sound: its width is the points along an edge.
    let table = asset.kind == MediaKind::Lut;
    let timed = asset.kind != MediaKind::Image && !table;
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&asset.name)
                        .font(theme::medium(13.0))
                        .color(p.fg),
                )
                .wrap(),
            );
            let mut job = egui::text::LayoutJob::default();
            let format = |font, color| egui::TextFormat {
                font_id: font,
                color,
                ..Default::default()
            };
            job.append(kind, 0.0, format(theme::regular(11.5), p.secondary));
            if timed {
                job.append(" · ", 0.0, format(theme::regular(11.5), p.secondary));
                job.append(
                    &timefmt::short(asset.duration),
                    0.0,
                    format(theme::mono(11.0), p.secondary),
                );
            }
            ui.add(egui::Label::new(job));
        });
    ui.add_space(12.0);
    // A lookup table is neither placed nor played.
    if crate::rules::placeable(asset) {
        button_row(ui, p, |ui| {
            if button(ui, p, Some(Icon::Plus), "Add to timeline", Kind::Plain).clicked() {
                actions.push(Action::InsertAsset {
                    asset: asset.id,
                    place: None,
                });
            }
            if button(ui, p, Some(Icon::Play), "Preview", Kind::Plain)
                .on_hover_text("Play this media without adding it (Enter)")
                .clicked()
            {
                actions.push(Action::PreviewAsset(asset.id));
            }
        });
    }
    section(ui, p, "Details");
    card(ui, p, |ui| {
        fact(ui, p, "Kind", kind);
        if timed {
            time_fact(ui, p, "Duration", timefmt::short(asset.duration));
        }
        if table {
            let edge = asset.width;
            fact(ui, p, "Table", format!("{edge}×{edge}×{edge}"));
        } else if asset.width > 0 && asset.height > 0 {
            fact(ui, p, "Size", format!("{}×{}", asset.width, asset.height));
        }
        if let Some(rate) = asset.frame_rate {
            fact(
                ui,
                p,
                "Frame rate",
                format!("{} fps", trim_number(rate.fps())),
            );
        }
        if timed {
            fact(ui, p, "Sound", if asset.has_audio { "Yes" } else { "No" });
        }
        if asset.file_size > 0 {
            fact(ui, p, "File", timefmt::file_size(asset.file_size));
        }
        note(ui, p, &asset.path.display().to_string());
    });
}

fn project_pane(ui: &mut Ui, p: &Palette, project: &Project, draft: &mut ProjectDraft) {
    let clips: usize = project.tracks.iter().map(|track| track.clips.len()).sum();
    section(ui, p, "Project");
    card(ui, p, |ui| {
        row(ui, p, "Name", |ui| {
            let mut name = draft.name.clone().unwrap_or_else(|| project.name.clone());
            let id = egui::Id::new("project-name");
            // As wide as the canvas's pair of fields and its rate under it,
            // so that the card's values start at one place.
            let width = (2.0 * paired_width(ui) + PAIRED_GAP).min(ui.available_width() - 4.0);
            if text_field(ui, p, id, &mut name, "Project name", width).changed() {
                draft.name = Some(name);
            }
        });
    });

    let mut canvas = draft
        .canvas
        .clone()
        .unwrap_or_else(|| project.settings.clone());
    let before = canvas.clone();
    section(ui, p, "Canvas");
    card(ui, p, |ui| {
        #[derive(Clone, Copy, PartialEq)]
        enum Shape {
            Wide,
            Tall,
            Square,
            Custom,
        }
        let mut shape = match (canvas.width, canvas.height) {
            (1920, 1080) => Shape::Wide,
            (1080, 1920) => Shape::Tall,
            (1080, 1080) => Shape::Square,
            _ => Shape::Custom,
        };
        // A size the presets do not name leaves all three unselected; the
        // fields below set it.
        let shapes = [
            (Shape::Wide, "16:9"),
            (Shape::Tall, "9:16"),
            (Shape::Square, "1:1"),
        ];
        if choice(ui, p, &mut shape, &shapes) {
            let size = CanvasSettings::preset(match shape {
                Shape::Tall => "9:16",
                Shape::Square => "1:1",
                Shape::Wide | Shape::Custom => "16:9",
            });
            canvas.width = size.width;
            canvas.height = size.height;
        }
        row(ui, p, "Size", |ui| {
            let width = paired_width(ui);
            ui.spacing_mut().item_spacing.x = PAIRED_GAP;
            let range = 16.0..=16384.0;
            let mut height = canvas.height as f32;
            let mut wide = canvas.width as f32;
            let field = |ui: &mut Ui, value: &mut f32, axis, name| {
                number_with(ui, p, value, &range, 2.0, (Some(axis), ""), 0, width, name).changed()
            };
            in_reading_order(ui, 2.0 * width + PAIRED_GAP, |ui| {
                if field(ui, &mut wide, "W", "Canvas width") {
                    canvas.width = wide.round() as u32;
                }
                if field(ui, &mut height, "H", "Canvas height") {
                    canvas.height = height.round() as u32;
                }
            });
        });
        let wide = 2.0 * paired_width(ui) + PAIRED_GAP;
        tall_row(ui, p, "Frame rate", |ui| {
            let mut rate = canvas.frame_rate;
            let named: Vec<(FrameRate, String)> = RATES
                .iter()
                .map(|(numerator, denominator, label)| {
                    let rate = FrameRate {
                        numerator: *numerator,
                        denominator: *denominator,
                    };
                    (rate, format!("{label} fps"))
                })
                .collect();
            let options: Vec<(FrameRate, &str)> = named
                .iter()
                .map(|(rate, label)| (*rate, label.as_str()))
                .collect();
            // A rate the list does not name is shown as it is.
            if !options.iter().any(|(listed, _)| *listed == rate) {
                let shown = format!("{} fps", trim_number(rate.fps()));
                let (_, list) =
                    select_menu(ui, p, ("frame-rate", "Frame rate"), &shown, wide, |ui| {
                        options.iter().find_map(|(listed, label)| {
                            kit::menu_row(ui, p, None, label, "", false, Some(false))
                                .clicked()
                                .then_some(*listed)
                        })
                    });
                if let Some(Some(listed)) = list {
                    rate = listed;
                }
            } else {
                select_list(ui, p, "frame-rate", "Frame rate", &mut rate, &options, wide);
            }
            canvas.frame_rate = rate;
        });
        row(ui, p, "Background", |ui| {
            if well::well(ui, p, "Canvas colour", &mut canvas.background, false, &[]).changed() {
                canvas.background[3] = 255;
            }
        });
    });
    if canvas != before {
        draft.canvas = Some(canvas);
    }

    section(ui, p, "Contents");
    card(ui, p, |ui| {
        fact(ui, p, "Tracks", project.tracks.len().to_string());
        fact(ui, p, "Clips", clips.to_string());
        fact(ui, p, "Media", project.assets.len().to_string());
        fact(ui, p, "Markers", project.markers.len().to_string());
    });
}
