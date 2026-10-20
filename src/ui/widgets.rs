//! Shared controls. Use these before adding a new visual pattern.
//!
//! Each is drawn by Neptune's shared vocabulary ([`super::controls`],
//! [`super::panel`] and [`super::kit`]) and takes as much room as it did
//! before, so an area keeps its layout until it is laid out again; it then
//! calls the primitive itself at its final size.
use super::{
    controls,
    kit::{self, RowFit},
};
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Color32, CursorIcon, FontId, Galley, Id, Rect, Response, Sense, Ui, Vec2, WidgetInfo,
    WidgetType, pos2, vec2,
};
use std::sync::Arc;

pub use super::kit::{KeyRow, KeyState};

/// The side of a control that sits inside a row beside a field.
pub const SMALL_BUTTON: f32 = metrics::SMALL_BUTTON;

/// A pane's well: the content surface, rounded, with a hairline inside its
/// edge.
pub fn panel(ui: &Ui, rect: Rect, p: &Palette) {
    kit::pane_well(ui.painter(), rect, p.bg, p);
}

/// One line of text cut to `width` with an ellipsis.
pub fn truncated(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    controls::elided(ui.painter(), text, font, color, width)
}

/// The same dropdown chevron as a select's, in egui's existing icon slot.
pub fn combo_icon(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _is_open: bool) {
    let side = rect.width().min(rect.height()).min(12.0);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(side)),
        Icon::ChevronDown,
        visuals.fg_stroke.color,
    );
}

fn name(response: &Response, enabled: bool, label: &str) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
}

/// An icon-only control: a 28-point target around a 16-point glyph, named by
/// its tooltip. A shortcut in a trailing "(…)" of its name is shown as keys.
pub fn icon_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    icon_button_tinted(ui, p, icon, label, None)
}

/// As [`icon_button`], with the glyph in `tint`, a state such as record. A
/// state that is merely on is a pressed surface, not an accent glyph.
pub fn icon_button_tinted(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    label: &str,
    tint: Option<Color32>,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::ICON_BUTTON), Sense::click());
    let on = tint == Some(p.accent);
    let tint = tint.filter(|_| !on);
    paint_icon_button(ui, p, rect, &response, icon, tint, on);
    name(&response, ui.is_enabled(), label);
    let (shown, keys) = kit::name_and_keys(label);
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        shown,
        keys,
    )
}

/// A toggle that stays pressed while it is on.
pub fn icon_toggle(ui: &mut Ui, p: &Palette, icon: Icon, on: bool, label: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::ICON_BUTTON), Sense::click());
    paint_icon_button(ui, p, rect, &response, icon, None, on);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    let (shown, keys) = kit::name_and_keys(label);
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        shown,
        keys,
    )
}

/// Paints an icon control into a rect the caller already allocated: the
/// glyph in `tint` for a state, and the surface pressed while `held`.
pub fn paint_icon_button(
    ui: &Ui,
    p: &Palette,
    rect: Rect,
    response: &Response,
    icon: Icon,
    tint: Option<Color32>,
    held: bool,
) {
    let mut response = response.clone();
    response.rect = rect;
    kit::paint_icon_button(ui, p, &response, icon, held, tint);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A secondary action: the control fill.
    Plain,
    /// The one recommended action in view.
    Primary,
    /// Ends processes or discards work.
    Destructive,
    /// Text only, for tertiary actions.
    #[allow(dead_code, reason = "offered to the areas as they are laid out again")]
    Quiet,
}

/// A labelled button, optionally led by an icon.
pub fn button(ui: &mut Ui, p: &Palette, icon: Option<Icon>, text: &str, kind: Kind) -> Response {
    let font = theme::medium(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    // The icon side takes two points less padding than the text side.
    let (lead, trail) = if icon.is_some() {
        (10.0, 12.0)
    } else {
        (12.0, 12.0)
    };
    let icon_width = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let size = vec2(
        lead + icon_width + galley.size().x + trail,
        metrics::legacy::CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    name(&response, ui.is_enabled(), text);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let enabled = ui.is_enabled();
    let (fill, ink) = match kind {
        Kind::Primary => (p.accent, p.on_accent),
        Kind::Destructive => (p.red, p.on_red()),
        Kind::Plain => (p.control, p.fg),
        Kind::Quiet => (Color32::TRANSPARENT, p.secondary),
    };
    let solid = matches!(kind, Kind::Primary | Kind::Destructive);
    let radius = metrics::CONTROL_RADIUS;
    let painter = ui.painter();
    // An action that cannot be taken keeps its kind at half strength; the
    // disabled interface fades it, so it is not drawn fainter again here.
    painter.rect_filled(rect, radius, fill);
    if enabled && response.is_pointer_button_down_on() {
        painter.rect_filled(
            rect,
            radius,
            if solid {
                Color32::from_black_alpha(46)
            } else {
                p.pressed
            },
        );
    } else if enabled && response.hovered() {
        painter.rect_filled(
            rect,
            radius,
            if solid {
                Color32::from_white_alpha(26)
            } else {
                p.hover
            },
        );
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, radius, *p);
    }
    let ink = if enabled || solid { ink } else { p.muted };
    let mut x = rect.left() + lead;
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
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A row of mutually exclusive choices with a thumb that slides to the one
/// chosen.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    // Named by its choices, which tell one apart from another in a row.
    let name = options
        .iter()
        .map(|(_, label)| *label)
        .collect::<Vec<_>>()
        .join("|");
    controls::segmented(ui, *p, &name, value, options, width)
}

/// A small heading over a group of rows.
pub fn section(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text)
            .font(theme::medium(11.5))
            .color(p.secondary),
    );
}

/// A label at the leading edge and a control at the trailing edge of one row.
pub fn row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    kit::row(ui, p, label, RowFit::LEGACY, add)
}

/// A hairline that spans the available width, set in from the leading edge
/// as the rows it separates are.
pub fn hairline(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    let left = (rect.left() + 12.0).min(rect.right());
    ui.painter()
        .hline(left..=rect.right(), rect.center().y, p.hairline());
}

/// A gutter between two panels that is also their resize handle. Returns the
/// drag along its axis; a double click asks for the default size.
pub fn splitter(ui: &mut Ui, p: &Palette, rect: Rect, id: Id, vertical: bool) -> (f32, bool) {
    kit::splitter(ui, p, rect, id, vertical)
}

/// The quiet statement an empty panel makes: an icon, a line and a hint.
pub fn empty_state(ui: &mut Ui, p: &Palette, rect: Rect, icon: Icon, title: &str, hint: &str) {
    kit::empty_state(ui, p, rect, icon, title, hint, None);
}

/// A number that is dragged or typed, styled as a field.
pub fn number(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    suffix: &str,
    decimals: usize,
) -> Response {
    let p = palette(ui);
    let number = kit::Number {
        range: &range,
        speed,
        suffix,
        decimals,
    };
    kit::number_field(ui, &p, value, number, vec2(78.0, 24.0), None)
}

/// The palette a control without one is drawn from, read back from the
/// interface's style.
fn palette(ui: &Ui) -> Palette {
    let visuals = ui.visuals();
    let mut p = Palette::new(
        if visuals.dark_mode {
            theme::Appearance::Dark
        } else {
            theme::Appearance::Light
        },
        theme::Accent::default(),
    );
    p.accent = visuals.selection.stroke.color;
    p
}

/// Styles the rows of a menu being built: a fixed width, rows set close.
pub fn menu_style(ui: &mut Ui, width: f32) {
    controls::menu_layout(ui, width);
    // Without a limit a menu takes the widest room its popup may have.
    ui.set_min_width(width);
    ui.set_max_width(width);
    ui.spacing_mut().button_padding = vec2(8.0, 5.0);
}

/// A menu row with its shortcut at the trailing edge.
pub fn menu_item(ui: &mut Ui, p: &Palette, icon: Option<Icon>, text: &str, keys: &str) -> Response {
    kit::menu_row(ui, p, icon, text, keys, false, None)
}

/// A menu row for an action that discards something: red, and red where
/// it is highlighted.
#[allow(dead_code, reason = "offered to the menus as they are laid out again")]
pub fn menu_item_destructive(
    ui: &mut Ui,
    p: &Palette,
    icon: Option<Icon>,
    text: &str,
    keys: &str,
) -> Response {
    kit::menu_row(ui, p, icon, text, keys, true, None)
}

/// An icon-only control for the inside of a row: a 22-point target around a
/// 14-point glyph, named by its tooltip.
pub fn small_icon_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    kit::small_icon_button(ui, p, icon, label)
}

/// A row whose value can be keyframed: the label at the leading edge, the
/// fields `add` makes at the trailing edge, and after them the steps to the
/// keyframes on either side of the playhead and the diamond, which stand at
/// the same place on every row. `what` names the value for the diamond's
/// accessible name.
pub fn keyed_row(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    what: &str,
    state: KeyState,
    add: impl FnOnce(&mut Ui),
) -> KeyRow {
    keyed_row_with(ui, p, label, what, state, add, |_| {})
}

/// As [`keyed_row`], with what `lead` makes before the fields.
pub fn keyed_row_with(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    what: &str,
    state: KeyState,
    add: impl FnOnce(&mut Ui),
    lead: impl FnOnce(&mut Ui),
) -> KeyRow {
    kit::keyed_row(ui, p, label, what, state, RowFit::LEGACY, add, lead)
}

/// As [`row`], with a label that is cut with an ellipsis rather than pushed
/// under its controls when the panel is narrow.
pub fn fitted_row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    kit::fitted_row(ui, p, label, RowFit::LEGACY, add)
}

/// As [`number`], named for accessibility: a field beside a label is
/// otherwise only its value.
#[expect(clippy::too_many_arguments, reason = "a field is all of these")]
pub fn named_number(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    suffix: &str,
    decimals: usize,
    width: f32,
    label: &str,
) -> Response {
    let p = palette(ui);
    let number = kit::Number {
        range: &range,
        speed,
        suffix,
        decimals,
    };
    kit::number_field(ui, &p, value, number, vec2(width, 24.0), Some(label))
}

/// A value from nought to one on a track, dragged or stepped with the up and
/// down arrows. A double click returns it to `default`.
pub fn slider(
    ui: &mut Ui,
    p: &Palette,
    value: &mut f32,
    default: f32,
    width: f32,
    label: &str,
) -> Response {
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(width, SMALL_BUTTON), Sense::click_and_drag());
    let before = *value;
    // The knob's centre travels between its own radius from each end.
    let knob_radius = 8.0;
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - knob_radius * 2.0, 4.0));
    if response.double_clicked() {
        *value = default;
    } else if (response.dragged() || response.is_pointer_button_down_on())
        && let Some(pos) = response.interact_pointer_pos()
    {
        *value = ((pos.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
    }
    if response.has_focus() {
        // The arrows are this control's while it has the focus.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let steps = ui.input(|i| {
            let size = if i.modifiers.shift { 0.1 } else { 0.01 };
            size * (i.num_presses(egui::Key::ArrowUp) as f32
                - i.num_presses(egui::Key::ArrowDown) as f32)
        });
        if steps != 0.0 {
            *value = (*value + steps).clamp(0.0, 1.0);
        }
    }
    if *value != before {
        response.mark_changed();
    }
    let shown = *value;
    response.widget_info(|| {
        let mut info = WidgetInfo::slider(ui.is_enabled(), f64::from(shown) * 100.0, label);
        info.label = Some(label.to_owned());
        info
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if ui.is_enabled() { p.accent } else { p.muted };
        // The track runs the whole width; the knob stops its radius short.
        let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
        painter.rect_filled(rail, 2, p.pressed);
        let knob = pos2(track.left() + track.width() * shown, track.center().y);
        painter.rect_filled(
            Rect::from_min_max(rail.min, pos2(knob.x, rail.bottom())),
            2,
            fill,
        );
        if response.has_focus() {
            painter.circle_stroke(
                knob,
                knob_radius + 2.0,
                egui::Stroke::new(2.0, theme::tint(p.accent, 0.55)),
            );
        }
        painter.circle_filled(
            knob + vec2(0.0, 1.0),
            knob_radius,
            Color32::from_black_alpha(50),
        );
        painter.circle_filled(knob, knob_radius, Color32::WHITE);
    }
    if ui.is_enabled() {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// As [`small_icon_button`], for a state: pressed while it is on.
pub fn small_icon_toggle(ui: &mut Ui, p: &Palette, icon: Icon, on: bool, label: &str) -> Response {
    kit::small_icon_toggle(ui, p, icon, on, label)
}

/// A switch for a setting that is on or off.
pub fn switch(ui: &mut Ui, p: &Palette, on: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, SMALL_BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    if ui.is_rect_visible(rect) {
        let progress = controls::animate(
            ui.ctx(),
            response.id.with("progress"),
            on,
            theme::motion::SWITCH,
        );
        let painter = ui.painter();
        // The track is translucent, so it reads on any surface; the accent
        // comes over it as the switch slides on.
        painter.rect_filled(rect, 11, p.pressed);
        painter.rect_filled(rect, 11, theme::tint(p.accent, progress));
        if response.has_focus() {
            controls::focus_ring(painter, rect, 11, *p);
        }
        let travel = rect.width() - 22.0;
        let knob = pos2(rect.left() + 11.0 + travel * progress, rect.center().y);
        painter.circle_filled(knob + vec2(0.0, 1.0), 9.0, Color32::from_black_alpha(40));
        painter.circle_filled(knob, 9.0, Color32::WHITE);
    }
    if ui.is_enabled() {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The heading of a group of rows that opens and closes: a chevron and a
/// name, cut with an ellipsis, over the width that is left.
pub fn disclosure(ui: &mut Ui, p: &Palette, open: bool, text: &str, dim: bool) -> Response {
    let size = vec2(ui.available_width().max(24.0), SMALL_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::CollapsingHeader, enabled, text);
        info.selected = Some(open);
        info
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_expanded(open);
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(rect, 6, p.pressed);
        } else if enabled && response.hovered() {
            painter.rect_filled(rect, 6, p.hover);
        }
        if response.has_focus() {
            controls::focus_ring(painter, rect, 6, *p);
        }
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(14.0)),
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            p.secondary,
        );
        let ink = if dim { p.muted } else { p.fg };
        let galley = truncated(ui, text, theme::medium(12.5), ink, rect.width() - 40.0);
        controls::galley_at(painter, pos2(rect.left() + 36.0, rect.center().y), galley);
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// How far a piece of background work has got: a 4-point bar over the width
/// available, filled in `fill`, named `name` with its value for those who do
/// not see it.
pub fn progress(ui: &mut Ui, p: &Palette, done: f32, fill: Color32, name: &str) -> Response {
    kit::progress_bar(ui, p, done, fill, name)
}

/// A pill that picks one of several named choices, such as a preset. It is
/// pressed while it is the one chosen.
pub fn chip(ui: &mut Ui, p: &Palette, text: &str, selected: bool) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), theme::medium(12.0), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 22.0, 26.0);
    let (_, response) = ui.allocate_exact_size(size, Sense::click());
    // The chip keeps the width it had; the check comes with the area's
    // final layout, which leaves room for it.
    kit::paint_chip(ui, p, &response, galley, selected, false);
    if ui.is_enabled() {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, Appearance};
    use eframe::egui::{
        Event, Modifiers, PointerButton, Pos2, Shape, epaint::RectShape, epaint::Stroke,
    };

    fn palette() -> Palette {
        Palette::new(Appearance::Dark, Accent::default())
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette());
        ctx
    }

    /// Runs one frame of `add` in a fresh 480 × 320 window and returns what
    /// it painted.
    fn frame(ctx: &egui::Context, events: Vec<Event>, add: &mut dyn FnMut(&mut Ui)) -> Vec<Shape> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 320.0))),
                events,
                ..Default::default()
            },
            |ui| add(ui),
        );
        output.textures_delta.clear();
        fn flatten(shape: Shape, into: &mut Vec<Shape>) {
            match shape {
                Shape::Vec(shapes) => shapes.into_iter().for_each(|shape| flatten(shape, into)),
                shape => into.push(shape),
            }
        }
        let mut shapes = Vec::new();
        for clipped in output.shapes {
            flatten(clipped.shape, &mut shapes);
        }
        shapes
    }

    fn rects(shapes: &[Shape]) -> impl Iterator<Item = &RectShape> {
        shapes.iter().filter_map(|shape| match shape {
            Shape::Rect(rect) => Some(rect),
            _ => None,
        })
    }

    #[test]
    fn every_adapter_takes_the_room_it_took_before() {
        // Spec §4.2 (F14): the adapters restyle without moving anything.
        let ctx = context();
        let p = palette();
        let mut sizes = Vec::new();
        frame(&ctx, Vec::new(), &mut |ui| {
            let text = ui
                .painter()
                .layout_no_wrap("Export".into(), theme::medium(12.5), Color32::WHITE)
                .size()
                .x;
            let chip_text = ui
                .painter()
                .layout_no_wrap("1080p".into(), theme::medium(12.0), Color32::WHITE)
                .size()
                .x;
            let mut on = 0.5;
            let mut value = 3.0;
            // A number field was egui's drag value, sized as asked.
            let bare = |ui: &mut Ui, width: f32| {
                ui.add_sized(
                    [width, 24.0],
                    egui::DragValue::new(&mut 3.0_f32)
                        .range(0.0..=10.0)
                        .suffix(" s")
                        .fixed_decimals(1),
                )
                .rect
                .size()
            };
            let (pair, single) = (bare(ui, 64.0), bare(ui, 78.0));
            let mut choice = 0;
            sizes = vec![
                (
                    button(ui, &p, None, "Export", Kind::Primary).rect.size(),
                    vec2(text + 24.0, 28.0),
                ),
                (
                    button(ui, &p, Some(Icon::Plus), "Export", Kind::Plain)
                        .rect
                        .size(),
                    vec2(text + 42.0, 28.0),
                ),
                (
                    icon_button(ui, &p, Icon::Undo, "Undo (Ctrl+Z)").rect.size(),
                    vec2(28.0, 28.0),
                ),
                (
                    icon_toggle(ui, &p, Icon::Loop, true, "Loop").rect.size(),
                    vec2(28.0, 28.0),
                ),
                (
                    small_icon_button(ui, &p, Icon::Rotate, "Reset").rect.size(),
                    vec2(22.0, 22.0),
                ),
                (
                    small_icon_toggle(ui, &p, Icon::Eye, true, "Show")
                        .rect
                        .size(),
                    vec2(22.0, 22.0),
                ),
                (switch(ui, &p, true, "Snap").rect.size(), vec2(32.0, 22.0)),
                (
                    slider(ui, &p, &mut on, 1.0, 112.0, "Mix").rect.size(),
                    vec2(112.0, 22.0),
                ),
                (
                    chip(ui, &p, "1080p", true).rect.size(),
                    vec2(chip_text + 22.0, 26.0),
                ),
                (
                    named_number(ui, &mut value, 0.0..=10.0, 0.1, " s", 1, 64.0, "Length")
                        .rect
                        .size(),
                    pair,
                ),
                (
                    number(ui, &mut value, 0.0..=10.0, 0.1, " s", 1).rect.size(),
                    single,
                ),
                (
                    disclosure(ui, &p, true, "Encoder", false).rect.size(),
                    vec2(ui.available_width(), 22.0),
                ),
            ];
            let before = ui.cursor().top();
            segmented(ui, &p, &mut choice, &[(0, "Dark"), (1, "Light")], 150.0);
            sizes.push((
                vec2(
                    150.0,
                    ui.cursor().top() - before - ui.spacing().item_spacing.y,
                ),
                vec2(150.0, 28.0),
            ));
        });
        for (index, (size, before)) in sizes.into_iter().enumerate() {
            assert!(
                (size - before).length() < 0.5,
                "adapter {index}: {size:?}, was {before:?}"
            );
        }
    }

    #[test]
    fn hover_and_press_fill_with_the_hover_and_pressed_tokens() {
        let ctx = context();
        let p = palette();
        let target = std::cell::Cell::new(Rect::NOTHING);
        let mut add = |ui: &mut Ui| {
            target.set(icon_button(ui, &p, Icon::Undo, "Undo").rect);
            button(ui, &p, None, "Keep", Kind::Plain);
        };
        frame(&ctx, Vec::new(), &mut add);
        let at = target.get().center();
        let hovered = frame(&ctx, vec![Event::PointerMoved(at)], &mut add);
        assert!(
            rects(&hovered).any(|rect| rect.fill == p.hover),
            "the hovered control lies under the hover fill"
        );
        let press = Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        let pressed = frame(&ctx, vec![press], &mut add);
        assert!(
            rects(&pressed).any(|rect| rect.fill == p.pressed),
            "the pressed control lies under the pressed fill"
        );
        assert!(!rects(&pressed).any(|rect| rect.fill == p.hover));
    }

    #[test]
    fn a_control_reached_with_the_keyboard_shows_the_focus_ring() {
        let ctx = context();
        let p = palette();
        let ring = Stroke::new(2.0, theme::tint(p.accent, 0.55));
        let id = std::cell::Cell::new(None);
        let mut add = |ui: &mut Ui| {
            let response = button(ui, &p, None, "Export", Kind::Plain);
            id.set(Some(response.id));
        };
        let resting = frame(&ctx, Vec::new(), &mut add);
        assert!(!rects(&resting).any(|rect| rect.stroke == ring));
        ctx.memory_mut(|memory| memory.request_focus(id.get().unwrap()));
        let focused = frame(&ctx, Vec::new(), &mut add);
        assert!(
            rects(&focused).any(|rect| rect.stroke == ring),
            "a focused button is ringed in the accent"
        );
    }

    #[test]
    fn a_shortcut_in_a_name_is_set_apart_for_the_tooltip() {
        assert_eq!(kit::name_and_keys("Undo (Ctrl+Z)"), ("Undo", "Ctrl+Z"));
        assert_eq!(
            kit::name_and_keys("Loop playback (Cmd+L)"),
            ("Loop playback", "Cmd+L")
        );
        assert_eq!(kit::name_and_keys("Render queue"), ("Render queue", ""));
        assert_eq!(kit::name_and_keys("Odd ()"), ("Odd ()", ""));
    }
}
