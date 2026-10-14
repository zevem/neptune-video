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
mod transition;

pub use color::{Channel, key_colour_command};

use super::{
    Action, ClipDraft, ColourPick, DuckProgress, Selection, SteadyShown, TrackProgress,
    TrackRegion, icon_of_media, tone_of_media,
    viewer::trim_number,
    widgets::{self, Kind, number, row, section},
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
    self, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, epaint::RectShape,
    pos2, vec2,
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
        }
    }
}

impl InspectorState {
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
    /// those keys are then its own, and not the timeline's.
    pub fn owns_keys(&self, ctx: &egui::Context) -> bool {
        (self.keys).is_some_and(|frame| frame + 1 >= ctx.cumulative_frame_nr())
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

/// A group of rows on the control material.
fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(p.control)
        .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS + 2))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            add(ui)
        })
        .inner
}

fn fact(ui: &mut Ui, p: &Palette, label: &str, value: impl Into<String>) {
    row(ui, p, label, |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(value.into())
                    .font(theme::regular(12.5))
                    .color(p.fg),
            )
            .truncate(),
        );
    });
}

fn title(ui: &mut Ui, p: &Palette, icon: Icon, tint: Color32, name: &str, detail: &str) {
    ui.horizontal(|ui| {
        let (tile, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
        ui.painter()
            .rect_filled(tile, CornerRadius::same(8), theme::tint(tint, 0.18));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(tile.center(), Vec2::splat(16.0)),
            icon,
            tint,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(
                egui::Label::new(
                    egui::RichText::new(name)
                        .font(theme::medium(13.0))
                        .color(p.fg),
                )
                .truncate(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(detail)
                        .font(theme::regular(11.5))
                        .color(p.muted),
                )
                .truncate(),
            );
        });
    });
    ui.add_space(6.0);
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
    // A clip's name and its sections stay where they are while its rows
    // scroll under them, and each section keeps its own place.
    if let Some((track, clip)) = selection.single().and_then(|id| project.clip(id)) {
        if let Some(edge) =
            (input.transition).filter(|edge| super::transitions::of(clip, *edge).is_some())
        {
            scrolled(ui, rect, ("inspector-transition", edge), 10, |ui| {
                transition::pane(ui, input, track, clip, edge, state, draft, actions);
            });
            return;
        }
        // A short panel gives the rows the room: the name is one line.
        let compact = rect.height() < COMPACT_HEAD;
        let top = Rect::from_min_max(
            rect.min + vec2(12.0, if compact { 4.0 } else { 10.0 }),
            pos2(rect.right() - 12.0, rect.bottom()),
        );
        let mut head = ui.new_child(egui::UiBuilder::new().max_rect(top));
        head.set_clip_rect(rect);
        head.spacing_mut().item_spacing.y = if compact { 4.0 } else { 6.0 };
        let section = clip::head(&mut head, input, track, clip, state, compact);
        let rows = Rect::from_min_max(
            pos2(rect.left(), head.min_rect().bottom().min(rect.bottom())),
            rect.max,
        );
        scrolled(ui, rows, ("inspector-clip", section), 4, |ui| {
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
        });
        return;
    }
    scrolled(ui, rect, "inspector", 10, |ui| {
        if selection.clips.len() > 1 {
            several_pane(ui, p, project, selection, actions);
        } else if let Some(asset) = selection.asset.and_then(|id| project.assets.get(&id)) {
            asset_pane(ui, p, asset, input.textures, actions);
        } else {
            project_pane(ui, p, project, project_draft);
        }
    });
}

/// The panel height under which a clip's name and sections are drawn small,
/// so that the rows under them keep most of it.
const COMPACT_HEAD: f32 = 300.0;

/// A clip's name and where it is, on one line.
fn compact_title(ui: &mut Ui, p: &Palette, name: &str, detail: &str) {
    let mut job = egui::text::LayoutJob::default();
    let format = |font, color| egui::TextFormat {
        font_id: font,
        color,
        ..Default::default()
    };
    job.append(name, 0.0, format(theme::medium(12.5), p.fg));
    job.append(detail, 8.0, format(theme::regular(11.5), p.muted));
    ui.add(egui::Label::new(job).truncate());
}

/// A pane's rows in `rect`, scrolled when there are more than fit. A row
/// the keyboard moves to is scrolled into view.
fn scrolled(
    ui: &mut Ui,
    rect: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
    top: i8,
    add: impl FnOnce(&mut Ui),
) {
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    body.set_clip_rect(rect);
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show(&mut body, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 12,
                    top,
                    bottom: 10,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    add(ui);
                    // The control the keyboard has, when it has just moved
                    // there without the pointer and is one of these rows.
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
                        let by_pointer =
                            ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
                        if let Some(row) = focused.and_then(|id| ui.ctx().read_response(id))
                            && !by_pointer
                            && ui.min_rect().contains(row.rect.center())
                        {
                            ui.scroll_to_rect(row.rect.expand2(vec2(0.0, 10.0)), None);
                        }
                    }
                });
        });
}

fn several_pane(
    ui: &mut Ui,
    p: &Palette,
    project: &Project,
    selection: &Selection,
    actions: &mut Vec<Action>,
) {
    title(
        ui,
        p,
        Icon::Layers,
        p.accent,
        &format!("{} clips selected", selection.clips.len()),
        "Edit them together",
    );
    let ids = selection.clips.clone();
    let any = |test: fn(&Clip) -> bool| {
        ids.iter()
            .any(|id| project.clip(*id).is_some_and(|(_, clip)| test(clip)))
    };
    ui.horizontal_wrapped(|ui| {
        if widgets::button(ui, p, Some(Icon::Layers), "Group", Kind::Plain).clicked() {
            actions.push(Action::Edit(Command::GroupClips {
                clip_ids: ids.clone(),
            }));
        }
        if widgets::button(ui, p, Some(Icon::Link), "Link", Kind::Plain).clicked() {
            actions.push(Action::Edit(Command::LinkClips {
                clip_ids: ids.clone(),
            }));
        }
        if any(|clip| clip.group_id.is_some())
            && widgets::button(ui, p, None, "Ungroup", Kind::Plain).clicked()
        {
            actions.push(Action::Edit(Command::UngroupClips {
                clip_ids: ids.clone(),
            }));
        }
        if any(|clip| clip.linked_id.is_some())
            && widgets::button(ui, p, None, "Unlink", Kind::Plain).clicked()
        {
            actions.push(Action::Edit(Command::UnlinkClips {
                clip_ids: ids.clone(),
            }));
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if widgets::button(ui, p, Some(Icon::Trash), "Delete", Kind::Plain).clicked() {
            actions.push(Action::Delete { ripple: false });
        }
        if widgets::button(ui, p, None, "Ripple delete", Kind::Plain).clicked() {
            actions.push(Action::Delete { ripple: true });
        }
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
    let radius = CornerRadius::same(metrics::CONTROL_RADIUS + 2);
    ui.painter().rect_filled(picture, radius, p.sunken);
    match textures.poster(asset.id) {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (picture.width() / size.x).min(picture.height() / size.y);
            let fitted = Rect::from_center_size(picture.center(), size * scale);
            ui.painter().add(
                RectShape::filled(fitted, CornerRadius::ZERO, Color32::WHITE).with_texture(
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
        Stroke::new(
            1.0,
            if p.dark {
                Color32::from_white_alpha(26)
            } else {
                Color32::from_black_alpha(26)
            },
        ),
        StrokeKind::Inside,
    );
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(&asset.name)
                .font(theme::medium(13.0))
                .color(p.fg),
        )
        .wrap(),
    );
    section(ui, p, "Details");
    card(ui, p, |ui| {
        fact(
            ui,
            p,
            "Kind",
            match asset.kind {
                MediaKind::Video => "Video",
                MediaKind::Audio => "Audio",
                MediaKind::Image => "Image",
                MediaKind::Gif => "GIF",
                MediaKind::Lut => "LUT",
            },
        );
        // A still has no length of its own, and a lookup table no time,
        // picture or sound: its width is the points along an edge.
        let table = asset.kind == MediaKind::Lut;
        if asset.kind != MediaKind::Image && !table {
            fact(ui, p, "Duration", timefmt::short(asset.duration));
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
        if asset.kind != MediaKind::Image && !table {
            fact(ui, p, "Sound", if asset.has_audio { "Yes" } else { "No" });
        }
        if asset.file_size > 0 {
            fact(ui, p, "File", timefmt::file_size(asset.file_size));
        }
    });
    ui.add(
        egui::Label::new(
            egui::RichText::new(asset.path.display().to_string())
                .font(theme::regular(11.0))
                .color(p.muted),
        )
        .wrap(),
    );
    ui.add_space(4.0);
    // A lookup table is neither placed nor played.
    if !crate::rules::placeable(asset) {
        return;
    }
    ui.horizontal(|ui| {
        if widgets::button(ui, p, Some(Icon::Plus), "Add to timeline", Kind::Primary).clicked() {
            actions.push(Action::InsertAsset {
                asset: asset.id,
                place: None,
            });
        }
        if widgets::button(ui, p, Some(Icon::Play), "Preview", Kind::Plain)
            .on_hover_text("Play this media without adding it (Enter)")
            .clicked()
        {
            actions.push(Action::PreviewAsset(asset.id));
        }
    });
}

fn project_pane(ui: &mut Ui, p: &Palette, project: &Project, draft: &mut ProjectDraft) {
    let clips: usize = project.tracks.iter().map(|track| track.clips.len()).sum();
    title(
        ui,
        p,
        Icon::Film,
        p.accent,
        &project.name,
        &format!(
            "{} · {}",
            timefmt::short(project.duration()),
            match clips {
                1 => "1 clip".to_owned(),
                n => format!("{n} clips"),
            }
        ),
    );

    section(ui, p, "Project");
    card(ui, p, |ui| {
        row(ui, p, "Name", |ui| {
            let mut name = draft.name.clone().unwrap_or_else(|| project.name.clone());
            let response = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .desired_width(150.0)
                    .font(theme::regular(12.5)),
            );
            if response.changed() {
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
        let width = ui.available_width();
        // A size the presets do not name leaves all three unselected; the
        // fields below set it.
        if widgets::segmented(
            ui,
            p,
            &mut shape,
            &[
                (Shape::Wide, "16:9"),
                (Shape::Tall, "9:16"),
                (Shape::Square, "1:1"),
            ],
            width,
        ) {
            let size = CanvasSettings::preset(match shape {
                Shape::Tall => "9:16",
                Shape::Square => "1:1",
                Shape::Wide | Shape::Custom => "16:9",
            });
            canvas.width = size.width;
            canvas.height = size.height;
        }
        ui.add_space(4.0);
        row(ui, p, "Size", |ui| {
            let mut height = canvas.height as f32;
            let mut width = canvas.width as f32;
            if number(ui, &mut height, 16.0..=16384.0, 2.0, " H", 0).changed() {
                canvas.height = height.round() as u32;
            }
            if number(ui, &mut width, 16.0..=16384.0, 2.0, " W", 0).changed() {
                canvas.width = width.round() as u32;
            }
        });
        row(ui, p, "Frame rate", |ui| {
            let current = RATES
                .iter()
                .find(|(n, d, _)| {
                    canvas.frame_rate.numerator == *n && canvas.frame_rate.denominator == *d
                })
                .map_or_else(
                    || trim_number(canvas.frame_rate.fps()),
                    |(_, _, label)| (*label).to_owned(),
                );
            egui::ComboBox::from_id_salt("frame-rate")
                .selected_text(format!("{current} fps"))
                .width(96.0)
                .show_ui(ui, |ui| {
                    for (numerator, denominator, label) in RATES {
                        let rate = FrameRate {
                            numerator,
                            denominator,
                        };
                        ui.selectable_value(&mut canvas.frame_rate, rate, format!("{label} fps"));
                    }
                });
        });
        row(ui, p, "Background", |ui| {
            let mut rgb = [
                canvas.background[0],
                canvas.background[1],
                canvas.background[2],
            ];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                canvas.background = [rgb[0], rgb[1], rgb[2], 255];
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
