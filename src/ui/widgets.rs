//! Shared controls. Use these before adding a new visual pattern.
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Galley, Id, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, text::LayoutJob, vec2,
};
use std::sync::Arc;

/// Fills a rounded panel in the window material.
pub fn panel(ui: &Ui, rect: Rect, p: &Palette) {
    ui.painter()
        .rect_filled(rect, CornerRadius::same(metrics::PANEL_RADIUS), p.surface);
}

/// One line of text cut to `width` with an ellipsis.
pub fn truncated(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// The same dropdown chevron as our menus, in egui's existing icon slot.
pub fn combo_icon(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _is_open: bool) {
    let side = rect.width().min(rect.height()).min(14.0);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(side)),
        Icon::ChevronDown,
        visuals.fg_stroke.color,
    );
}

/// The fill behind a hovered or pressed control, faded over 120 ms. A press
/// also scales the control to 0.96 so that it answers the finger.
fn feedback(ui: &Ui, response: &Response, rect: Rect, p: &Palette, radius: u8) -> Rect {
    let ctx = ui.ctx();
    let pressed = response.is_pointer_button_down_on() && response.enabled();
    let hover = ctx.animate_bool_with_time(
        response.id.with("hover"),
        response.hovered() && response.enabled(),
        0.12,
    );
    let press = ctx.animate_bool_with_time(response.id.with("press"), pressed, 0.1);
    let rect = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.04 * press));
    if hover > 0.0 || press > 0.0 {
        let fill = if press > 0.0 { p.pressed } else { p.hover };
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius),
            fill.gamma_multiply(hover.max(press)),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    rect
}

fn name(response: &Response, enabled: bool, label: &str) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
}

/// An icon-only control: a 28-point target around a 16-point glyph, named by
/// its tooltip.
pub fn icon_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    icon_button_tinted(ui, p, icon, label, None)
}

/// As [`icon_button`], with the glyph in `tint` (a state, such as a toggle
/// that is on).
pub fn icon_button_tinted(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    label: &str,
    tint: Option<Color32>,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::ICON_BUTTON), Sense::click());
    paint_icon_button(ui, p, rect, &response, icon, tint, false);
    name(&response, ui.is_enabled(), label);
    response
        .on_hover_cursor(CursorIcon::Default)
        .on_hover_text(label)
}

/// A toggle that stays filled while it is on.
pub fn icon_toggle(ui: &mut Ui, p: &Palette, icon: Icon, on: bool, label: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::ICON_BUTTON), Sense::click());
    paint_icon_button(ui, p, rect, &response, icon, on.then_some(p.accent), on);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    response.on_hover_text(label)
}

/// Paints an icon control into a rect the caller already allocated.
pub fn paint_icon_button(
    ui: &Ui,
    p: &Palette,
    rect: Rect,
    response: &Response,
    icon: Icon,
    tint: Option<Color32>,
    held: bool,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    if held {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(metrics::CONTROL_RADIUS),
            theme::tint(p.accent, 0.16),
        );
    }
    let rect = feedback(ui, response, rect, p, metrics::CONTROL_RADIUS);
    let color = if !ui.is_enabled() {
        p.muted.gamma_multiply(0.6)
    } else if let Some(tint) = tint {
        tint
    } else if response.hovered() {
        p.fg
    } else {
        p.secondary
    };
    let glyph = Rect::from_center_size(rect.center(), Vec2::splat(metrics::ICON));
    icons::paint(ui.painter(), glyph, icon, color);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Primary,
    Destructive,
}

/// A labelled button, optionally led by an icon.
pub fn button(ui: &mut Ui, p: &Palette, icon: Option<Icon>, text: &str, kind: Kind) -> Response {
    let font = theme::medium(12.5);
    let ink = match kind {
        Kind::Plain => p.fg,
        Kind::Primary => p.on_accent,
        Kind::Destructive => Color32::WHITE,
    };
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
        metrics::CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    name(&response, ui.is_enabled(), text);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let enabled = ui.is_enabled();
    let press = ui.ctx().animate_bool_with_time(
        response.id.with("press"),
        response.is_pointer_button_down_on() && enabled,
        0.1,
    );
    let body = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.04 * press));
    let base = match kind {
        Kind::Plain => p.control,
        Kind::Primary => p.accent,
        Kind::Destructive => p.red,
    };
    let fill = if !enabled {
        // An action that cannot be taken is a plain control, not a fainter
        // accent.
        p.control
    } else if kind == Kind::Plain {
        if response.is_pointer_button_down_on() {
            p.pressed
        } else if response.hovered() {
            p.pressed.gamma_multiply(0.8)
        } else {
            base
        }
    } else if response.hovered() {
        theme::mix(base, Color32::WHITE, 0.08)
    } else {
        base
    };
    let painter = ui.painter();
    painter.rect_filled(body, CornerRadius::same(metrics::CONTROL_RADIUS), fill);
    if response.has_focus() {
        painter.rect_stroke(
            body.expand(2.0),
            CornerRadius::same(metrics::CONTROL_RADIUS + 2),
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    let ink = if enabled { ink } else { p.muted };
    let mut x = body.left() + lead;
    if let Some(icon) = icon {
        let glyph = Rect::from_center_size(pos2(x + 7.0, body.center().y), Vec2::splat(14.0));
        icons::paint(painter, glyph, icon, ink);
        x += icon_width;
    }
    painter.galley(
        pos2(x, body.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    response
}

/// A row of mutually exclusive choices on one track.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let height = metrics::CONTROL_HEIGHT;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(metrics::CONTROL_RADIUS), p.control);
    let inner = rect.shrink(2.0);
    let cell = inner.width() / options.len().max(1) as f32;
    let mut changed = false;
    for (index, (option, label)) in options.iter().enumerate() {
        let cell_rect = Rect::from_min_size(
            pos2(inner.left() + cell * index as f32, inner.top()),
            vec2(cell, inner.height()),
        );
        let id = ui.id().with(("segment", *label, index));
        let response = ui.interact(cell_rect, id, Sense::click());
        let selected = *value == *option;
        let enabled = ui.is_enabled();
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, *label)
        });
        if selected {
            // Concentric with the track: 8 less the 2-point inset.
            ui.painter().rect_filled(
                cell_rect,
                CornerRadius::same(metrics::CONTROL_RADIUS - 2),
                if p.dark { p.pressed } else { p.elevated },
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                cell_rect,
                CornerRadius::same(metrics::CONTROL_RADIUS - 2),
                Stroke::new(1.5, p.accent),
                StrokeKind::Inside,
            );
        }
        ui.painter().text(
            cell_rect.center(),
            Align2::CENTER_CENTER,
            *label,
            theme::medium(12.0),
            if selected || response.hovered() {
                p.fg
            } else {
                p.secondary
            },
        );
        if response.clicked() && !selected {
            *value = *option;
            changed = true;
        }
    }
    changed
}

/// A small uppercase-free section heading, as a settings group has.
pub fn section(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text)
            .font(theme::medium(11.5))
            .color(p.muted),
    );
}

/// A label at the leading edge and a control at the trailing edge of one row.
pub fn row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::CONTROL_HEIGHT);
        ui.label(
            egui::RichText::new(label)
                .font(theme::regular(12.5))
                .color(p.secondary),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
            .inner
    })
    .inner
}

/// A hairline that spans the available width.
pub fn hairline(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, p.hairline());
}

/// A gutter between two panels that is also their resize handle. Returns the
/// drag along its axis; a double click asks for the default size.
pub fn splitter(ui: &mut Ui, p: &Palette, rect: Rect, id: Id, vertical: bool) -> (f32, bool) {
    // A gutter is the pointer's: it is not a place Tab stops at, where it
    // would hold the keyboard and show and do nothing.
    let response = ui.interact(rect.expand(1.0), id, Sense::CLICK | Sense::DRAG);
    let cursor = if vertical {
        CursorIcon::ResizeHorizontal
    } else {
        CursorIcon::ResizeVertical
    };
    let active = response.hovered() || response.dragged();
    if active {
        ui.ctx().set_cursor_icon(cursor);
    }
    let shown = ui
        .ctx()
        .animate_bool_with_time(id.with("grip"), active, 0.12);
    if shown > 0.0 {
        let grip = if vertical {
            Rect::from_center_size(rect.center(), vec2(2.0, 36.0))
        } else {
            Rect::from_center_size(rect.center(), vec2(36.0, 2.0))
        };
        ui.painter()
            .rect_filled(grip, CornerRadius::same(1), p.border.gamma_multiply(shown));
    }
    let delta = if response.dragged() {
        if vertical {
            response.drag_delta().x
        } else {
            response.drag_delta().y
        }
    } else {
        0.0
    };
    (delta, response.double_clicked())
}

/// The quiet statement an empty panel makes: an icon, a line and a hint.
pub fn empty_state(ui: &mut Ui, p: &Palette, rect: Rect, icon: Icon, title: &str, hint: &str) {
    let center = rect.center();
    let painter = ui.painter_at(rect);
    icons::paint(
        &painter,
        Rect::from_center_size(center - vec2(0.0, 30.0), Vec2::splat(28.0)),
        icon,
        p.muted,
    );
    painter.text(
        center + vec2(0.0, 2.0),
        Align2::CENTER_CENTER,
        title,
        theme::medium(13.0),
        p.secondary,
    );
    let galley = painter.layout(
        hint.to_owned(),
        theme::regular(12.0),
        p.muted,
        (rect.width() - 32.0).max(40.0),
    );
    painter.galley(
        pos2(center.x - galley.size().x * 0.5, center.y + 16.0),
        galley,
        p.muted,
    );
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
    ui.add_sized(
        [78.0, 24.0],
        egui::DragValue::new(value)
            .range(range)
            .speed(speed)
            .suffix(suffix)
            .fixed_decimals(decimals),
    )
}

/// Styles the rows of a menu being built: fixed width and quiet text.
pub fn menu_style(ui: &mut Ui, width: f32) {
    ui.set_min_width(width);
    // Without a limit a menu takes the widest room its popup may have.
    ui.set_max_width(width);
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.spacing_mut().button_padding = vec2(8.0, 5.0);
}

/// A menu row with its shortcut at the trailing edge.
pub fn menu_item(ui: &mut Ui, p: &Palette, icon: Option<Icon>, text: &str, keys: &str) -> Response {
    let width = ui.available_width().max(ui.min_rect().width());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    name(&response, ui.is_enabled(), text);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let enabled = ui.is_enabled();
    // The row the arrows are on is shown as the one the pointer is on.
    let highlighted = (response.hovered() || response.has_focus()) && enabled;
    let painter = ui.painter();
    if highlighted {
        painter.rect_filled(rect, CornerRadius::same(6), p.accent);
    }
    let ink = if !enabled {
        p.muted
    } else if highlighted {
        p.on_accent
    } else {
        p.fg
    };
    let mut x = rect.left() + 8.0;
    if let Some(icon) = icon {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(14.0)),
            icon,
            ink,
        );
    }
    x += 22.0;
    painter.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::regular(13.0),
        ink,
    );
    if !keys.is_empty() {
        painter.text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            keys,
            theme::regular(12.0),
            if highlighted { p.on_accent } else { p.muted },
        );
    }
    response
}

/// The side of a control that sits inside a row beside a field.
pub const SMALL_BUTTON: f32 = 22.0;

/// An icon-only control for the inside of a row: a 22-point target around a
/// 14-point glyph, named by its tooltip.
pub fn small_icon_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SMALL_BUTTON), Sense::click());
    name(&response, ui.is_enabled(), label);
    if ui.is_rect_visible(rect) {
        let rect = feedback(ui, &response, rect, p, 6);
        let color = if !ui.is_enabled() {
            p.muted.gamma_multiply(0.5)
        } else if response.hovered() {
            p.fg
        } else {
            p.secondary
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
            icon,
            color,
        );
    }
    response
        .on_hover_cursor(CursorIcon::Default)
        .on_hover_text(label)
}

/// What a row's keyframe diamond says of its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// The stored value is the one shown at every time.
    Off,
    /// Keyframes drive the value. `on_key` is whether the playhead sits on
    /// one; `previous` and `next` whether there is one to go to.
    Animated {
        on_key: bool,
        previous: bool,
        next: bool,
    },
}

/// The controls [`keyed_row`] drew beside its fields.
pub struct KeyRow {
    pub toggle: Response,
    /// The steps to the keyframe before and after the playhead, on a row
    /// keyframes drive.
    pub previous: Option<Response>,
    pub next: Option<Response>,
}

/// The keyframe diamond: an outline while the value is not animated, the
/// accent once it is, and filled while the playhead sits on a keyframe.
pub fn key_toggle(ui: &mut Ui, p: &Palette, state: KeyState, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SMALL_BUTTON), Sense::click());
    let animated = state != KeyState::Off;
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), animated, label)
    });
    if ui.is_rect_visible(rect) {
        let rect = feedback(ui, &response, rect, p, 6);
        let ink = if !ui.is_enabled() {
            p.muted.gamma_multiply(0.5)
        } else if animated {
            p.accent
        } else if response.hovered() {
            p.fg
        } else {
            p.muted
        };
        let on_key = matches!(state, KeyState::Animated { on_key: true, .. });
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
            if on_key {
                Icon::DiamondFilled
            } else {
                Icon::Diamond
            },
            ink,
        );
    }
    response
        .on_hover_cursor(CursorIcon::Default)
        .on_hover_text(label)
}

/// A row whose value can be keyframed: the label at the leading edge, cut
/// with an ellipsis when the panel is narrow, and at the trailing edge the
/// fields `add` makes with the keyframe diamond beside them. A row keyframes
/// drive also has the steps to the keyframes on either side of the playhead.
/// `what` names the value for the diamond's accessible name.
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

/// As [`keyed_row`], with what `lead` makes between the label and the
/// keyframe controls, so that the diamond keeps its place when a row gains
/// or loses a control.
pub fn keyed_row_with(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    what: &str,
    state: KeyState,
    add: impl FnOnce(&mut Ui),
    lead: impl FnOnce(&mut Ui),
) -> KeyRow {
    // The row is made in reading order, which is the order Tab walks it in:
    // the label, then the steps and the diamond, then the fields. The label
    // is cut to what the controls leave, and how wide they are is known only
    // once they are made: it is kept from the frame before, and a row whose
    // controls changed width is laid out again before it is shown.
    let id = ui.next_auto_id().with("keyed-row");
    let known: Option<f32> = ui.data(|data| data.get_temp(id));
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::CONTROL_HEIGHT);
        let gap = ui.spacing().item_spacing.x;
        let controls = known.unwrap_or(3.0 * SMALL_BUTTON + 86.0);
        let room = (ui.available_width() - controls - 4.0).max(0.0);
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(
            vec2(room, metrics::CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(room);
                ui.set_max_width(room);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(label)
                            .font(theme::regular(12.5))
                            .color(p.secondary),
                    )
                    .truncate(),
                );
            },
        );
        ui.add_space(4.0);
        let from = ui.cursor().left();
        lead(ui);
        let step = |ui: &mut Ui, icon, enabled, label: &str| {
            ui.add_enabled_ui(enabled, |ui| small_icon_button(ui, p, icon, label))
                .inner
        };
        let (previous, next) = match state {
            KeyState::Off => (None, None),
            KeyState::Animated { previous, next, .. } => (Some(previous), Some(next)),
        };
        let previous = previous.map(|there| {
            step(
                ui,
                Icon::ChevronLeft,
                there,
                &format!("Previous {what} keyframe"),
            )
        });
        let toggle = key_toggle(
            ui,
            p,
            state,
            &match state {
                KeyState::Off => format!("Animate {what}"),
                KeyState::Animated { on_key: true, .. } => {
                    format!("Remove {what} keyframe")
                }
                KeyState::Animated { .. } => format!("Add {what} keyframe"),
            },
        );
        let next = next.map(|there| {
            step(
                ui,
                Icon::ChevronRight,
                there,
                &format!("Next {what} keyframe"),
            )
        });
        ui.spacing_mut().item_spacing.x = gap;
        ui.add_space(gap);
        // The fields are named by the row and by nothing made before them in
        // it. A row gains and loses its reset and its steps while a value is
        // typed, and a field named by its place among them would be another
        // field from then on: it would lose the keyboard part way through.
        ui.scope_builder(egui::UiBuilder::new().id(id.with("fields")), add);
        let used = ui.min_rect().right() - from;
        if known.is_none_or(|known| (known - used).abs() > 0.5) {
            ui.data_mut(|data| data.insert_temp(id, used));
            ui.ctx()
                .request_discard("a keyed row's controls changed width");
        }
        KeyRow {
            toggle,
            previous,
            next,
        }
    })
    .inner
}

/// As [`row`], with a label that is cut with an ellipsis rather than pushed
/// under its controls when the panel is narrow.
pub fn fitted_row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::CONTROL_HEIGHT);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let inner = add(ui);
            ui.add_space(4.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(label)
                            .font(theme::regular(12.5))
                            .color(p.secondary),
                    )
                    .truncate(),
                );
            });
            inner
        })
        .inner
    })
    .inner
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
    let response = ui.add_sized(
        [width, 24.0],
        egui::DragValue::new(value)
            .range(range)
            .speed(speed)
            .suffix(suffix)
            .fixed_decimals(decimals),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::DragValue, ui.is_enabled(), label));
    response
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
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 12.0, 4.0));
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
        let enabled = ui.is_enabled();
        let fill = if enabled { p.accent } else { p.muted };
        painter.rect_filled(track, CornerRadius::same(2), p.pressed);
        let knob = pos2(track.left() + track.width() * shown, track.center().y);
        painter.rect_filled(
            Rect::from_min_max(track.min, pos2(knob.x, track.bottom())),
            CornerRadius::same(2),
            fill,
        );
        let held = response.dragged() || response.is_pointer_button_down_on();
        painter.circle_filled(knob, if held { 6.5 } else { 5.5 }, Color32::WHITE);
        painter.circle_stroke(
            knob,
            if held { 6.5 } else { 5.5 },
            Stroke::new(1.0, p.border),
        );
        if response.has_focus() {
            painter.circle_stroke(knob, 8.0, Stroke::new(1.5, p.accent));
        }
    }
    response.on_hover_cursor(CursorIcon::Default)
}

/// As [`small_icon_button`], for a state: the glyph takes the accent while it
/// is on.
pub fn small_icon_toggle(ui: &mut Ui, p: &Palette, icon: Icon, on: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SMALL_BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    if ui.is_rect_visible(rect) {
        let rect = feedback(ui, &response, rect, p, 6);
        let color = if !ui.is_enabled() {
            p.muted.gamma_multiply(0.5)
        } else if on {
            p.accent
        } else {
            p.muted
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
            icon,
            color,
        );
    }
    response
        .on_hover_cursor(CursorIcon::Default)
        .on_hover_text(label)
}

/// A switch for a setting that is on or off.
pub fn switch(ui: &mut Ui, p: &Palette, on: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, SMALL_BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    if ui.is_rect_visible(rect) {
        let body = Rect::from_center_size(rect.center(), vec2(30.0, 18.0));
        let slid = ui.ctx().animate_bool_with_time(response.id, on, 0.12);
        let painter = ui.painter();
        // The track is translucent, so it reads on any surface; the accent
        // comes over it as the switch slides on.
        if ui.is_enabled() {
            painter.rect_filled(body, CornerRadius::same(9), p.pressed);
            painter.rect_filled(body, CornerRadius::same(9), p.accent.gamma_multiply(slid));
        } else {
            painter.rect_filled(body, CornerRadius::same(9), p.control);
        }
        painter.circle_filled(
            pos2(body.left() + 9.0 + 12.0 * slid, body.center().y),
            7.0,
            Color32::WHITE,
        );
        if response.has_focus() {
            painter.rect_stroke(
                body.expand(2.0),
                CornerRadius::same(11),
                Stroke::new(1.5, p.accent),
                StrokeKind::Inside,
            );
        }
    }
    response.on_hover_cursor(CursorIcon::Default)
}

/// The heading of a group of rows that opens and closes: a chevron and a
/// name, cut with an ellipsis, over the width that is left.
pub fn disclosure(ui: &mut Ui, p: &Palette, open: bool, text: &str, dim: bool) -> Response {
    let size = vec2(ui.available_width().max(24.0), SMALL_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::CollapsingHeader, ui.is_enabled(), text);
        info.selected = Some(open);
        info
    });
    if ui.is_rect_visible(rect) {
        let rect = feedback(ui, &response, rect, p, 6);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), Vec2::splat(12.0)),
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            p.muted,
        );
        let ink = if dim { p.muted } else { p.fg };
        let galley = truncated(ui, text, theme::medium(12.5), ink, rect.width() - 24.0);
        ui.painter().galley(
            pos2(rect.left() + 20.0, rect.center().y - galley.size().y * 0.5),
            galley,
            ink,
        );
    }
    response.on_hover_cursor(CursorIcon::Default)
}

/// How far a piece of background work has got: a 4-point bar over the width
/// available, filled in `fill`, named `name` with its value for those who do
/// not see it.
pub fn progress(ui: &mut Ui, p: &Palette, done: f32, fill: Color32, name: &str) -> Response {
    let done = if done.is_finite() {
        done.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (bar, response) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, true, name);
        info.value = Some(f64::from(done));
        info
    });
    if ui.is_rect_visible(bar) {
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::same(2), p.pressed);
        if done > 0.0 {
            let mut filled = bar;
            // A started bar shows at least its rounded end.
            filled.set_width((bar.width() * done).max(4.0).min(bar.width()));
            painter.rect_filled(filled, CornerRadius::same(2), fill);
        }
    }
    response
}

/// A pill that picks one of several named choices, such as a preset. It is
/// filled with a tint of the accent while it is the one chosen.
pub fn chip(ui: &mut Ui, p: &Palette, text: &str, selected: bool) -> Response {
    let font = theme::medium(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 22.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, text));
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(13);
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, radius, theme::tint(p.accent, 0.18));
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, theme::tint(p.accent, 0.7)),
                StrokeKind::Inside,
            );
        } else {
            painter.rect_filled(rect, radius, p.control);
        }
        let body = feedback(ui, &response, rect, p, 13);
        let ink = if !enabled {
            p.muted.gamma_multiply(0.7)
        } else if selected || response.hovered() {
            p.fg
        } else {
            p.secondary
        };
        ui.painter().galley(
            pos2(
                body.center().x - galley.size().x * 0.5,
                body.center().y - galley.size().y * 0.5,
            ),
            galley,
            ink,
        );
    }
    response.on_hover_cursor(CursorIcon::Default)
}
