//! Shared controls and surfaces, ported from Neptune Terminal so that the
//! Neptune apps share one vocabulary. Each paints from palette tokens, keeps a
//! stable semantic identity and reports a native accessible role, so the custom
//! chrome stays keyboard-reachable and inspectable.
//!
//! Keep these as Terminal has them. Where Neptune Video departs, the code
//! says so with a `Neptune Video:` comment.

use super::kit;
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Frame, Galley, Id, Layout, Margin, Painter,
    Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType,
    vec2,
};
use std::{ops::RangeInclusive, sync::Arc};

/// Window-local state for a control whose public identity predates viewports.
/// Keep original-window IDs stable for callers and inspection fixtures.
pub fn viewport_id(ctx: &egui::Context, id: Id) -> Id {
    let viewport = ctx.viewport_id();
    if viewport == egui::ViewportId::ROOT {
        id
    } else {
        id.with(("viewport", viewport))
    }
}

/// Ease-out progress of a boolean state. Repaints only while it is moving, so
/// a settled interface stays idle.
pub fn animate(ctx: &egui::Context, id: Id, on: bool, seconds: f32) -> f32 {
    // Neptune Video: with Reduce motion on, every change settles at once.
    let seconds = if kit::reduced_motion(ctx) {
        0.0
    } else {
        seconds
    };
    // Context's animation manager is shared by every native viewport. A
    // sheet opening in one window must not animate another window's scrim.
    ctx.animate_bool_with_time_and_easing(
        id.with(ctx.viewport_id()),
        on,
        seconds,
        egui::emath::easing::cubic_out,
    )
}

/// Lays out controls inside an already allocated rectangle. Unlike a scope, it
/// leaves the parent's cursor alone, so a row keeps its declared height.
pub fn place<R>(
    ui: &mut Ui,
    rect: Rect,
    layout: Layout,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(Id::new(salt))
            .max_rect(rect)
            .layout(layout),
    );
    add_contents(&mut child)
}

/// One row of text, truncated with an ellipsis at `max_width`.
pub fn elided(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    painter.layout_job(job)
}

/// Paints a galley with its vertical centre on `left_center`.
pub fn galley_at(painter: &Painter, left_center: Pos2, galley: Arc<Galley>) -> Rect {
    let pos = Pos2::new(left_center.x, left_center.y - galley.size().y * 0.5);
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, Color32::PLACEHOLDER);
    rect
}

/// Splits a chord into its keys. A trailing `+` is the plus key itself.
pub(crate) fn chord_keys(shortcut: &str) -> Vec<&str> {
    let (modifiers, plus) = match shortcut.strip_suffix('+') {
        Some(rest) => (rest.strip_suffix('+').unwrap_or(rest), true),
        None => (shortcut, false),
    };
    modifiers
        .split('+')
        .filter(|key| !key.is_empty())
        .chain(plus.then_some("+"))
        .collect()
}

/// Paints right-aligned keycaps for a shortcut and returns their total width.
pub fn keycaps(painter: &Painter, right_center: Pos2, shortcut: &str, text: Color32) -> f32 {
    if shortcut.is_empty() {
        return 0.0;
    }
    let font = theme::medium(10.5);
    let mut right = right_center.x;
    for key in chord_keys(shortcut).into_iter().rev() {
        let galley = painter.layout_no_wrap(key.to_owned(), font.clone(), text);
        let width = (galley.size().x + 10.0).max(20.0);
        let cap = Rect::from_min_size(
            Pos2::new(right - width, right_center.y - 9.0),
            vec2(width, 18.0),
        );
        painter.rect_filled(cap, 5, theme::tint(text, 0.12));
        painter.galley(
            cap.center() - galley.size() * 0.5,
            galley,
            Color32::PLACEHOLDER,
        );
        right -= width + 3.0;
    }
    right_center.x - right - 3.0
}

/// The accent outline of a control that holds keyboard focus.
pub fn focus_ring(painter: &Painter, rect: Rect, radius: u8, p: Palette) {
    painter.rect_stroke(
        rect.expand(2.0),
        radius.saturating_add(2),
        Stroke::new(2.0, theme::tint(p.accent, 0.55)),
        StrokeKind::Inside,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one recommended action of a surface.
    Primary,
    Secondary,
    /// Ends processes or discards work.
    Destructive,
    /// Text-only, for tertiary actions.
    Quiet,
}

/// How large a text button is.
// Neptune Video: inspector rows take a compact button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    #[default]
    Regular,
    /// 26 tall, for the inside of a 32-point inspector row.
    Compact,
}

pub fn button(ui: &mut Ui, p: Palette, label: &str, kind: ButtonKind) -> Response {
    button_sized(ui, p, label, kind, ButtonSize::Regular)
}

/// As [`button`], at `size`.
// Neptune Video: `button` with its metrics taken from `size`.
pub fn button_sized(
    ui: &mut Ui,
    p: Palette,
    label: &str,
    kind: ButtonKind,
    size: ButtonSize,
) -> Response {
    let (font, padding, least, height) = match size {
        ButtonSize::Regular => (
            13.0,
            if kind == ButtonKind::Quiet {
                10.0
            } else {
                16.0
            },
            72.0,
            metrics::CONTROL_HEIGHT,
        ),
        ButtonSize::Compact => (
            12.5,
            if kind == ButtonKind::Quiet { 8.0 } else { 12.0 },
            56.0,
            metrics::INSPECTOR_CONTROL,
        ),
    };
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), theme::medium(font), Color32::PLACEHOLDER);
    let size = vec2((galley.size().x + padding * 2.0).max(least), height);
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, ui.id().with(("button", label)), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let down = response.is_pointer_button_down_on();
        let (fill, text) = match kind {
            ButtonKind::Primary => (p.accent, p.on_accent),
            ButtonKind::Destructive => (p.red, p.on_red()),
            ButtonKind::Secondary => (p.control, p.fg),
            ButtonKind::Quiet => (Color32::TRANSPARENT, p.secondary),
        };
        // Neptune Video: a disabled Secondary or Quiet button's words are
        // `muted`, as a disabled chip's and menu row's are, so every
        // disabled control reads at one strength once the disabled painter
        // halves it (spec F12). A solid button keeps its ink on its fill.
        let solid = matches!(kind, ButtonKind::Primary | ButtonKind::Destructive);
        let text = if ui.is_enabled() || solid {
            text
        } else {
            p.muted
        };
        let radius = metrics::CONTROL_RADIUS;
        let painter = ui.painter();
        painter.rect_filled(rect, radius, fill);
        if down {
            painter.rect_filled(
                rect,
                radius,
                if solid {
                    Color32::from_black_alpha(46)
                } else {
                    p.pressed
                },
            );
        } else if response.hovered() {
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
            focus_ring(painter, rect, radius, p);
        }
        painter.galley(rect.center() - galley.size() * 0.5, galley, text);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A switch for a setting that applies immediately.
pub fn toggle(ui: &mut Ui, p: Palette, on: &mut bool, label: &str) -> Response {
    let (_, rect) = ui.allocate_space(vec2(38.0, 22.0));
    let mut response = ui.interact(rect, ui.id().with(("toggle", label)), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let state = *on;
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), state, label));
    if ui.is_rect_visible(rect) {
        let progress = animate(ui.ctx(), response.id.with("progress"), state, 0.16);
        let painter = ui.painter();
        painter.rect_filled(rect, 11, p.pressed);
        painter.rect_filled(rect, 11, theme::tint(p.accent, progress));
        if response.has_focus() {
            focus_ring(painter, rect, 11, p);
        }
        let travel = rect.width() - 22.0;
        let knob = Pos2::new(rect.left() + 11.0 + travel * progress, rect.center().y);
        painter.circle_filled(knob + vec2(0.0, 1.0), 9.0, Color32::from_black_alpha(40));
        painter.circle_filled(knob, 9.0, Color32::WHITE);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A segmented control whose Left and Right also move the choice while one
/// of its cells has the keyboard, as a tab strip's do (Terminal). Whether
/// the choice changed.
pub fn segmented_choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: Palette,
    name: &str,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    // The ids `segmented` gives its cells.
    let id = ui.id().with(("segmented", name));
    let cells = (options.iter()).map(|(_, label)| id.with(*label));
    let selected = (options.iter())
        .position(|(option, _)| option == value)
        .unwrap_or(usize::MAX);
    let arrowed = super::arrow_among(ui.ctx(), cells, selected);
    if let Some(to) = arrowed {
        *value = options[to].0;
    }
    segmented(ui, p, name, value, options, width) || arrowed.is_some()
}

/// Mutually exclusive choices with a thumb that slides to the selection.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: Palette,
    name: &str,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let (_, rect) = ui.allocate_space(vec2(width, 28.0));
    let id = ui.id().with(("segmented", name));
    // Neptune Video: the labels fit their cells (spec §5.4). They are
    // Medium 12 while each has room on both sides of an equal cell, then
    // 11.5; past that, the cells share the width by their labels, each
    // with the same room on both sides, as an apportioned control does.
    let (cells, size) = segment_cells(ui, rect, options);
    // Neptune Video: a value none of the options holds selects none. No
    // option is shown or announced as chosen, and every one can be taken.
    let selected = options.iter().position(|(option, _)| option == value);
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 8, p.control);
    // Neptune Video: with Reduce motion on, the thumb moves at once.
    let slide = if kit::reduced_motion(ui.ctx()) {
        0.0
    } else {
        0.14
    };
    if let Some(chosen) = selected.and_then(|selected| cells.clone().nth(selected)) {
        let left = ui
            .ctx()
            .animate_value_with_time(id.with("thumb"), chosen.left(), slide);
        let wide = ui
            .ctx()
            .animate_value_with_time(id.with("thumb-width"), chosen.width(), slide);
        let thumb = Rect::from_min_size(
            Pos2::new(left, rect.top() + 2.0),
            vec2(wide, rect.height() - 4.0),
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
    }
    let mut changed = false;
    for (index, ((option, label), cell)) in options.iter().zip(cells).enumerate() {
        let cell = Rect::from_x_y_ranges(cell.x_range(), rect.y_range());
        let response = ui.interact(cell, id.with(*label), Sense::click());
        let active = Some(index) == selected;
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), active, *label)
        });
        if response.clicked() && !active {
            *value = *option;
            changed = true;
        }
        if response.has_focus() {
            focus_ring(&painter, cell.shrink(2.0), 6, p);
        }
        painter.text(
            cell.center(),
            Align2::CENTER_CENTER,
            *label,
            theme::medium(size),
            if active || response.hovered() {
                p.fg
            } else {
                p.secondary
            },
        );
        response.on_hover_cursor(CursorIcon::PointingHand);
    }
    changed
}

/// Neptune Video: the room a segmented label keeps on each side of it.
const SEGMENT_ROOM: f32 = 8.0;

/// Neptune Video: the least room on each side of a label at 11.5 that still
/// keeps a segmented control's cells equal, as the inspector's section
/// switch keeps its own. Below it the cells share the width by label.
pub const SEGMENT_SNUG: f32 = 6.0;

/// Neptune Video: the narrowest a segmented control of `labels` may be,
/// where each label at 11.5 still keeps [`SEGMENT_ROOM`] on both sides.
pub fn segmented_fit(ui: &Ui, labels: &[&str]) -> f32 {
    (labels.iter())
        .map(|label| label_width(ui, label, 11.5) + SEGMENT_ROOM * 2.0)
        .sum::<f32>()
        + 4.0
}

/// Neptune Video: how wide `label` is in Medium at `size`. A segmented
/// control fits its labels in every frame it is drawn in, so each width is
/// kept from the first frame that laid the label out, for the window's
/// scale, and later frames lay none out again.
fn label_width(ui: &Ui, label: &str, size: f32) -> f32 {
    let scale = ui.ctx().pixels_per_point();
    let key = Id::new(("segment-label", label, size.to_bits(), scale.to_bits()));
    if let Some(width) = ui.ctx().data(|data| data.get_temp::<f32>(key)) {
        return width;
    }
    let width = (ui.painter())
        .layout_no_wrap(label.to_owned(), theme::medium(size), Color32::PLACEHOLDER)
        .size()
        .x;
    ui.ctx().data_mut(|data| data.insert_temp(key, width));
    width
}

/// Neptune Video: the cells of a segmented control in `rect`, inside its
/// 2-point track, from leading to trailing, and the size their labels are
/// written at: equal cells at Medium 12, or at 11.5, while every label has
/// [`SEGMENT_ROOM`] on both sides, or at 11.5 while each still has
/// [`SEGMENT_SNUG`]; otherwise at 11.5 in cells as wide as their labels, the
/// width left over shared out evenly.
fn segment_cells<'a, T>(
    ui: &'a Ui,
    rect: Rect,
    options: &'a [(T, &str)],
) -> (impl Iterator<Item = Rect> + Clone + 'a, f32) {
    let inner = Rect::from_min_max(rect.min + vec2(2.0, 2.0), rect.max - vec2(2.0, 2.0));
    let count = options.len().max(1) as f32;
    let equal = inner.width() / count;
    let fits = |size: f32, room: f32| {
        (options.iter()).all(|(_, label)| label_width(ui, label, size) + room * 2.0 <= equal)
    };
    let even = [
        (12.0, SEGMENT_ROOM),
        (11.5, SEGMENT_ROOM),
        (11.5, SEGMENT_SNUG),
    ]
    .into_iter()
    .find(|&(size, room)| fits(size, room))
    .map(|(size, _)| size);
    let spare = match even {
        Some(_) => 0.0,
        None => {
            let labels = (options.iter())
                .map(|(_, label)| label_width(ui, label, 11.5))
                .sum::<f32>();
            (inner.width() - labels).max(0.0) / count
        }
    };
    let cells = options.iter().scan(inner.left(), move |left, (_, label)| {
        let wide = match even {
            Some(_) => equal,
            None => label_width(ui, label, 11.5) + spare,
        };
        let cell = Rect::from_min_size(Pos2::new(*left, inner.top()), vec2(wide, inner.height()));
        *left += wide;
        Some(cell)
    });
    (cells, even.unwrap_or(11.5))
}

/// A continuous value with a filled track. Arrow keys nudge by one step.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Terminal's vocabulary, which its test exercises")
)]
pub fn slider(
    ui: &mut Ui,
    p: Palette,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    width: f32,
) -> Response {
    let (_, rect) = ui.allocate_space(vec2(width, 22.0));
    let mut response = ui.interact(
        rect,
        ui.id().with(("slider", label)),
        Sense::click_and_drag(),
    );
    let (low, high) = (*range.start(), *range.end());
    let knob_radius = 8.0;
    let left = rect.left() + knob_radius;
    let travel = (rect.width() - knob_radius * 2.0).max(1.0);
    let snap = |raw: f32| ((raw / step).round() * step).clamp(low, high);
    let mut next = *value;
    if let Some(pos) = response.interact_pointer_pos() {
        next = snap(low + (high - low) * ((pos.x - left) / travel).clamp(0.0, 1.0));
    }
    if response.has_focus() {
        // Neptune Video: the arrows it moves with do not step the playhead.
        super::ChoiceArrows::hold(ui.ctx(), response.id);
        let nudge = ui.input(|input| {
            input.num_presses(egui::Key::ArrowRight) as f32
                - input.num_presses(egui::Key::ArrowLeft) as f32
        });
        if nudge != 0.0 {
            next = snap(*value + nudge * step);
        }
    }
    if next != *value {
        *value = next;
        response.mark_changed();
    }
    let current = *value;
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), current as f64, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fraction = ((current - low) / (high - low).max(f32::EPSILON)).clamp(0.0, 1.0);
        let knob = Pos2::new(left + travel * fraction, rect.center().y);
        let track = Rect::from_min_max(
            Pos2::new(rect.left(), rect.center().y - 2.0),
            Pos2::new(rect.right(), rect.center().y + 2.0),
        );
        painter.rect_filled(track, 2, p.pressed);
        painter.rect_filled(
            Rect::from_min_max(track.min, Pos2::new(knob.x, track.bottom())),
            2,
            p.accent,
        );
        if response.has_focus() {
            painter.circle_stroke(
                knob,
                knob_radius + 2.0,
                Stroke::new(2.0, theme::tint(p.accent, 0.55)),
            );
        }
        painter.circle_filled(
            knob + vec2(0.0, 1.0),
            knob_radius,
            Color32::from_black_alpha(50),
        );
        painter.circle_filled(knob, knob_radius, Color32::WHITE);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A bounded number adjusted in fixed steps. Returns whether it changed.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Terminal's vocabulary, which its test exercises")
)]
pub fn stepper(
    ui: &mut Ui,
    p: Palette,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    text: &str,
) -> bool {
    let (_, rect) = ui.allocate_space(vec2(112.0, 28.0));
    let id = ui.id().with(("stepper", label));
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 8, p.control);
    let mut changed = false;
    for (direction, icon, name, x) in [
        (-1.0, Icon::Minus, "Decrease", rect.left()),
        (1.0, Icon::Plus, "Increase", rect.right() - 30.0),
    ] {
        let cell = Rect::from_min_size(Pos2::new(x, rect.top()), vec2(30.0, rect.height()));
        let next = (*value + direction * step).clamp(*range.start(), *range.end());
        let enabled = next != *value;
        let response = ui.interact(cell, id.with(name), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Button,
                enabled,
                format!("{name} {}", label.to_lowercase()),
            )
        });
        if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(cell.shrink(2.0), 6, p.pressed);
        } else if enabled && response.hovered() {
            painter.rect_filled(cell.shrink(2.0), 6, p.hover);
        }
        if response.has_focus() {
            focus_ring(&painter, cell.shrink(2.0), 6, p);
        }
        icons::paint(
            &painter,
            Rect::from_center_size(cell.center(), Vec2::splat(12.0)),
            icon,
            if !enabled {
                p.muted
            } else if response.hovered() {
                p.fg
            } else {
                p.secondary
            },
        );
        if response.clicked() && enabled {
            *value = next;
            changed = true;
        }
        if enabled {
            response.on_hover_cursor(CursorIcon::PointingHand);
        }
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::medium(12.5),
        p.fg,
    );
    changed
}

/// The surface of an editable field: filled, with an accent ring while the
/// editor `id` holds the keyboard. A field whose text cannot be used is
/// outlined in the problem colour instead.
pub fn field_frame(ui: &Ui, p: Palette, rect: Rect, id: Id, valid: bool) {
    field_surface(ui, p, rect, id, (valid, metrics::CONTROL_RADIUS));
}
/// The same surface with another corner, for a field that holds controls
/// of its own and so is rounded concentrically around them.
pub fn field_surface(ui: &Ui, p: Palette, rect: Rect, id: Id, (valid, radius): (bool, u8)) {
    let id = viewport_id(ui.ctx(), id);
    let focused = ui.memory(|memory| memory.has_focus(id));
    let painter = ui.painter();
    painter.rect_filled(rect, radius, p.control);
    let ink = if valid { p.accent } else { p.red };
    if focused {
        painter.rect_stroke(
            rect.expand(2.5),
            radius.saturating_add(2),
            Stroke::new(3.0, theme::tint(ink, 0.28)),
            StrokeKind::Inside,
        );
    }
    let outline = if focused || !valid {
        Stroke::new(1.0, ink)
    } else {
        p.hairline()
    };
    painter.rect_stroke(rect, radius, outline, StrokeKind::Inside);
}

/// A filled single-line field with an accent focus ring. `label` is its
/// accessible name; the caller owns `id` so focus can be requested by identity.
pub fn text_field(
    ui: &mut Ui,
    p: Palette,
    id: Id,
    text: &mut String,
    hint: &str,
    label: &str,
    width: f32,
) -> Response {
    let (_, rect) = ui.allocate_space(vec2(width, metrics::CONTROL_HEIGHT));
    field_frame(ui, p, rect, id, true);
    let inner = rect.shrink2(vec2(10.0, 0.0));
    let response = place(
        ui,
        inner,
        Layout::left_to_right(Align::Center),
        ("text-field", id),
        |ui| bare_text_edit(ui, id, text, hint, 13.0, inner.width()),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, label));
    response
}

/// A search field, as Terminal draws it: a magnifier leads the text and a
/// clear control trails it. `label` is its accessible name; the caller owns
/// `id` so it can take focus.
pub fn search_field(
    ui: &mut Ui,
    p: Palette,
    id: Id,
    text: &mut String,
    hint: &str,
    label: &str,
    width: f32,
) -> Response {
    search_field_sized(
        ui,
        p,
        id,
        text,
        hint,
        label,
        vec2(width, metrics::CONTROL_HEIGHT),
    )
}

/// As [`search_field`], `size` large.
// Neptune Video: the library's search row is 28 points tall and
// Preferences' 30, so the field is drawn at the height it is given.
pub fn search_field_sized(
    ui: &mut Ui,
    p: Palette,
    id: Id,
    text: &mut String,
    hint: &str,
    label: &str,
    size: Vec2,
) -> Response {
    let (_, rect) = ui.allocate_space(size);
    field_frame(ui, p, rect, id, true);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + 16.0, rect.center().y),
            Vec2::splat(13.0),
        ),
        Icon::Search,
        p.muted,
    );
    let clearable = !text.is_empty();
    let inner = Rect::from_min_max(
        Pos2::new(rect.left() + 30.0, rect.top()),
        Pos2::new(
            rect.right() - if clearable { 30.0 } else { 10.0 },
            rect.bottom(),
        ),
    );
    let mut response = place(
        ui,
        inner,
        Layout::left_to_right(Align::Center),
        ("search-text", id),
        |ui| bare_text_edit(ui, id, text, hint, 13.0, inner.width()),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, label));
    if clearable {
        let target = Rect::from_center_size(
            Pos2::new(rect.right() - 16.0, rect.center().y),
            Vec2::splat(22.0),
        );
        let clear = ui.interact(target, id.with("clear"), Sense::click());
        clear.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Clear search"));
        let painter = ui.painter();
        if clear.hovered() || clear.has_focus() {
            painter.rect_filled(target.shrink(1.0), 6, p.hover);
        }
        if clear.has_focus() {
            focus_ring(painter, target.shrink(1.0), 6, p);
        }
        icons::paint(
            painter,
            Rect::from_center_size(target.center(), Vec2::splat(9.0)),
            Icon::Close,
            if clear.hovered() { p.fg } else { p.secondary },
        );
        if clear
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Clear search")
            .clicked()
        {
            text.clear();
            response.mark_changed();
            response.request_focus();
        }
    }
    response
}

/// A frameless single-line editor for surfaces that draw their own field.
pub fn bare_text_edit(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    hint: &str,
    size: f32,
    width: f32,
) -> Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .id(viewport_id(ui.ctx(), id))
            .hint_text(hint)
            .font(theme::regular(size))
            .frame(Frame::NONE)
            .background_color(Color32::TRANSPARENT)
            .margin(Margin::ZERO)
            .desired_width(width),
    )
}

/// Dims the window behind a sheet. Follows the window's rounded shape so the
/// transparent corners stay clear.
pub fn scrim(painter: &Painter, bounds: Rect, radius: u8, p: Palette, opacity: f32) {
    if opacity > 0.0 {
        painter.rect_filled(bounds, radius, p.scrim.gamma_multiply(opacity));
    }
}

/// A modal surface. Keyboard focus stays inside it, and it is exposed to
/// assistive tools as a window named `title`.
// Neptune Video: Terminal's `sheet`, centred, under the id the editor has
// always given each sheet, which its tests and notices find it by. It hands
// back egui's whole response, so that each sheet answers Escape, Enter and a
// click beside it as it always has; `kit::sheet_scrim` dims the window.
pub fn sheet<R>(
    ctx: &egui::Context,
    p: Palette,
    id: Id,
    title: &str,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::ModalResponse<R> {
    let screen = ctx.content_rect();
    let width = width.min(screen.width() - 24.0).max(120.0);
    let frame = Frame::new()
        .fill(p.elevated)
        .corner_radius(metrics::SHEET_RADIUS)
        .stroke(Stroke::new(1.0, p.border))
        .shadow(p.sheet_shadow());
    let modal = egui::Modal::new(id)
        .frame(frame)
        // The window paints its own rounded scrim.
        .backdrop_color(Color32::TRANSPARENT)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            add_contents(ui)
        });
    modal
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Window, true, title));
    modal
}

/// The title row of a sheet. Returns true when its close control is activated.
pub fn sheet_header(ui: &mut Ui, p: Palette, title: &str, close_label: Option<&str>) -> bool {
    header(ui, p, title, None, close_label).1
}

fn header(
    ui: &mut Ui,
    p: Palette,
    title: &str,
    back_label: Option<&str>,
    close_label: Option<&str>,
) -> (bool, bool) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 52.0));
    let control = |ui: &mut Ui, x: f32, icon: Icon, label: &str| {
        place(
            ui,
            Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::splat(28.0)),
            Layout::left_to_right(Align::Center),
            ("sheet-header", label),
            |ui| icons::button(ui, icon, label).clicked(),
        )
    };
    // Created before the close control, so Tab reaches it first.
    let back =
        back_label.is_some_and(|label| control(ui, rect.left() + 26.0, Icon::ChevronLeft, label));
    ui.painter().text(
        Pos2::new(
            rect.left() + if back_label.is_some() { 46.0 } else { 20.0 },
            rect.center().y,
        ),
        Align2::LEFT_CENTER,
        title,
        theme::semibold(15.0),
        p.fg,
    );
    let close =
        close_label.is_some_and(|label| control(ui, rect.right() - 26.0, Icon::Close, label));
    (back, close)
}

/// The action bar that closes a sheet, set off by a hairline. Returns the area
/// for its controls: tertiary actions lead, the confirming action trails.
pub fn sheet_footer(ui: &mut Ui, p: Palette) -> Rect {
    let (_, footer) = ui.allocate_space(vec2(ui.available_width(), 60.0));
    ui.painter()
        .line_segment([footer.left_top(), footer.right_top()], p.hairline());
    footer.shrink2(vec2(16.0, 0.0))
}

/// Left padding, content, right padding: the standard sheet gutter.
pub fn padded<R>(ui: &mut Ui, horizontal: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width() - horizontal * 2.0;
    ui.horizontal(|ui| {
        ui.add_space(horizontal);
        ui.vertical(|ui| {
            ui.set_width(width);
            add_contents(ui)
        })
        .inner
    })
    .inner
}

/// A small label above a group of related settings.
pub fn section_label(ui: &mut Ui, p: Palette, text: &str) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 24.0));
    ui.painter().text(
        Pos2::new(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::medium(11.5),
        p.secondary,
    );
}

/// An inset card of setting rows separated by hairlines.
pub struct Group {
    p: Palette,
    rows: usize,
}

pub fn group(ui: &mut Ui, p: Palette, add_rows: impl FnOnce(&mut Ui, &mut Group)) {
    let background = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().top();
    let width = ui.available_width();
    let left = ui.cursor().left();
    let mut group = Group { p, rows: 0 };
    ui.vertical(|ui| {
        ui.set_width(width);
        add_rows(ui, &mut group);
    });
    let rect = Rect::from_min_max(
        Pos2::new(left, top),
        Pos2::new(left + width, ui.cursor().top()),
    );
    ui.painter()
        .set(background, egui::Shape::rect_filled(rect, 10, p.control));
}

impl Group {
    fn allocate_row(&mut self, ui: &mut Ui, height: f32) -> Rect {
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), height));
        if self.rows > 0 {
            ui.painter().line_segment(
                [
                    Pos2::new(rect.left() + 14.0, rect.top()),
                    Pos2::new(rect.right(), rect.top()),
                ],
                self.p.hairline(),
            );
        }
        self.rows += 1;
        rect
    }

    /// A row the caller draws itself, set off from its neighbours like any other.
    pub fn custom(&mut self, ui: &mut Ui, height: f32) -> Rect {
        self.allocate_row(ui, height)
    }

    /// A full-width accordion header with an explicit expanded state.
    pub fn disclosure(&mut self, ui: &mut Ui, label: &str, open: &mut bool) -> Response {
        let rect = self.allocate_row(ui, 42.0);
        let mut response = ui.interact(
            rect,
            ui.id().with(("group-disclosure", label)),
            Sense::click(),
        );
        if response.clicked() {
            *open = !*open;
            response.mark_changed();
        }
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::CollapsingHeader, ui.is_enabled(), label)
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_expanded(*open);
        });
        if ui.is_rect_visible(rect) {
            let surface = rect.shrink2(vec2(4.0, 4.0));
            if response.is_pointer_button_down_on() {
                ui.painter().rect_filled(surface, 6, self.p.pressed);
            } else if response.hovered() {
                ui.painter().rect_filled(surface, 6, self.p.hover);
            }
            if response.has_focus() {
                focus_ring(ui.painter(), surface, 6, self.p);
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(
                    Pos2::new(rect.left() + 20.0, rect.center().y),
                    Vec2::splat(14.0),
                ),
                if *open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                self.p.secondary,
            );
            ui.painter().text(
                Pos2::new(rect.left() + 36.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                theme::regular(13.0),
                self.p.fg,
            );
        }
        response.on_hover_cursor(CursorIcon::PointingHand)
    }

    pub fn row_with_height<R>(
        &mut self,
        ui: &mut Ui,
        label: &str,
        height: f32,
        add_control: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let rect = self.allocate_row(ui, height);
        ui.painter().text(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::regular(13.0),
            self.p.fg,
        );
        place(
            ui,
            rect.shrink2(vec2(12.0, 0.0)),
            Layout::right_to_left(Align::Center),
            ("group-row", label),
            add_control,
        )
    }

    /// Supporting text inside the card, beneath the rows it explains.
    pub fn note(&mut self, ui: &mut Ui, text: &str) {
        let width = ui.available_width();
        let galley = ui.painter().layout(
            text.to_owned(),
            theme::regular(11.5),
            self.p.secondary,
            width - 28.0,
        );
        let (_, rect) = ui.allocate_space(vec2(width, galley.size().y + 20.0));
        if self.rows > 0 {
            ui.painter().line_segment(
                [
                    Pos2::new(rect.left() + 14.0, rect.top()),
                    Pos2::new(rect.right(), rect.top()),
                ],
                self.p.hairline(),
            );
        }
        self.rows += 1;
        ui.painter().galley(
            Pos2::new(rect.left() + 14.0, rect.top() + 10.0),
            galley,
            Color32::PLACEHOLDER,
        );
    }
}

fn menu_row(
    ui: &mut Ui,
    p: Palette,
    icon: Icon,
    label: &str,
    shortcut: &str,
    destructive: bool,
) -> Response {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 28.0));
    let response = ui.interact(rect, ui.id().with(("menu-item", label)), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let highlighted = response.hovered() || response.has_focus();
    let painter = ui.painter();
    let (text, hint) = if highlighted {
        painter.rect_filled(rect, 6, if destructive { p.red } else { p.accent });
        let ink = if destructive { p.on_red() } else { p.on_accent };
        (ink, ink)
    } else if destructive {
        (p.red, p.muted)
    } else {
        (p.fg, p.muted)
    };
    icons::paint(
        painter,
        Rect::from_center_size(
            Pos2::new(rect.left() + 15.0, rect.center().y),
            Vec2::splat(14.0),
        ),
        icon,
        text,
    );
    galley_at(
        painter,
        Pos2::new(rect.left() + 32.0, rect.center().y),
        elided(
            painter,
            label,
            theme::regular(13.0),
            text,
            rect.width()
                - 42.0
                - if shortcut.is_empty() {
                    0.0
                } else {
                    painter
                        .layout_no_wrap(shortcut.into(), theme::regular(11.5), hint)
                        .size()
                        .x
                        + 8.0
                },
        ),
    );
    if !shortcut.is_empty() {
        painter.text(
            Pos2::new(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            shortcut,
            theme::regular(11.5),
            hint,
        );
    }
    response
}

/// A submenu uses the same icon, text inset and hit area as other menu rows.
pub fn menu_submenu(
    ui: &mut Ui,
    p: Palette,
    icon: Icon,
    label: &str,
    content: impl FnOnce(&mut Ui),
) {
    let response = menu_row(ui, p, icon, label, "", false);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(response.rect.right() - 12.0, response.rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ChevronRight,
        if response.hovered() || response.has_focus() {
            p.on_accent
        } else {
            p.muted
        },
    );
    // Neptune Video: Escape in an open submenu closes it alone and leaves
    // the menu it opened from, with the keyboard back on its row. egui
    // closes the whole menu on that Escape, so it is taken here first.
    use egui::containers::menu::{MenuState, SubMenu};
    let id = SubMenu::id_from_widget_id(response.id);
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(id));
    // Only the deepest: a submenu that has one of its own open leaves the
    // Escape to it.
    let deepest = open && MenuState::from_id(ui.ctx(), id, |state| state.open_item.is_none());
    // Closed so, it opens again from the keyboard, or once the pointer
    // moves: one resting on its row does not open it again at once.
    let escaped = id.with("escaped");
    let pointer = ui.ctx().pointer_hover_pos();
    if deepest && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        MenuState::from_ui(ui, |state, _| state.open_item = None);
        ui.data_mut(|data| data.insert_temp(escaped, pointer));
        response.request_focus();
    }
    let mut row = response.clone();
    match ui.data(|data| data.get_temp::<Option<Pos2>>(escaped)) {
        Some(at) if at == pointer => row.rect = Rect::NOTHING,
        Some(_) => ui.data_mut(|data| data.remove::<Option<Pos2>>(escaped)),
        None => {}
    }
    SubMenu::new().show(ui, &row, content);
}

pub fn menu_separator(ui: &mut Ui, p: Palette) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 9.0));
    ui.painter().line_segment(
        [
            Pos2::new(rect.left() + 8.0, rect.center().y),
            Pos2::new(rect.right() - 8.0, rect.center().y),
        ],
        p.hairline(),
    );
}

/// Shared geometry for pop-up menus built from [`menu_item`]. The width is
/// fixed: a justified menu would otherwise grow to the widest space offered.
pub fn menu_layout(ui: &mut Ui, width: f32) {
    ui.set_width(width);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
}

/// A floating capsule, used for status inside a pane.
pub fn capsule(painter: &Painter, rect: Rect, p: Palette) {
    let radius = (rect.height() * 0.5).round() as u8;
    painter.add(p.popup_shadow().as_shape(rect, radius));
    painter.rect_filled(rect, radius, p.elevated);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
}

/// A setting stepped from the keyboard or the command palette, named with its
/// new value for a moment at the top of the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub name: &'static str,
    pub value: String,
    /// When the value was set, in the context's time.
    pub shown: f64,
}

impl Level {
    /// How long the chip stays before it fades.
    const HOLD: f64 = 1.2;
    const FADE: f64 = 0.16;

    /// Opacity at `now`, or `None` once the chip has gone.
    pub fn opacity(&self, now: f64) -> Option<f32> {
        let age = now - self.shown;
        (age < Self::HOLD + Self::FADE).then(|| {
            let fading = ((age - Self::HOLD) / Self::FADE).clamp(0.0, 1.0);
            1.0 - egui::emath::easing::cubic_out(fading as f32)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, Appearance};
    use eframe::egui::{Event, Modifiers, PointerButton};

    #[test]
    fn sibling_editors_keep_separate_cursor_state_and_original_public_ids() {
        let ctx = context();
        let child = egui::ViewportId::from_hash_of("editor-child");
        let public = Id::new("palette-input");
        let mut ids = Vec::new();
        for (viewport, value, selected) in [
            (egui::ViewportId::ROOT, "root text", 4),
            (child, "child text", 2),
        ] {
            let mut input = egui::RawInput {
                viewport_id: viewport,
                ..Default::default()
            };
            input.viewports.insert(child, egui::ViewportInfo::default());
            let mut editor_id = None;
            let mut output = ctx.run_ui(input, |ui| {
                let mut text = value.to_owned();
                let response = bare_text_edit(ui, public, &mut text, "", 13.0, 200.0);
                assert_eq!(response.id, viewport_id(ui.ctx(), public));
                let mut state =
                    egui::text_edit::TextEditState::load(ui.ctx(), response.id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(selected),
                    )));
                state.store(ui.ctx(), response.id);
                editor_id = Some(response.id);
            });
            output.textures_delta.clear();
            ids.push(editor_id.unwrap());
        }
        assert_eq!(ids[0], public);
        assert_ne!(ids[0], ids[1]);
        for (id, selected) in ids.into_iter().zip([4, 2]) {
            let state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
            let cursor = state.cursor.char_range().unwrap();
            assert_eq!(
                cursor.primary.index.max(cursor.secondary.index),
                egui::text::CharIndex(selected)
            );
        }
    }

    #[test]
    fn sibling_windows_can_keep_opposite_scrim_animation_states() {
        let ctx = egui::Context::default();
        let child = egui::ViewportId::from_hash_of("scrim-child");
        for (viewport, dim, time) in [
            (egui::ViewportId::ROOT, false, 0.0),
            (child, true, 0.016),
            (egui::ViewportId::ROOT, false, 0.032),
            (child, true, 0.048),
        ] {
            let mut input = egui::RawInput {
                viewport_id: viewport,
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 400.0))),
                ..Default::default()
            };
            input.viewports.insert(child, egui::ViewportInfo::default());
            let mut opacity = 0.0;
            let mut output = ctx.run_ui(input, |ui| {
                opacity = animate(ui.ctx(), Id::new("overlay-scrim"), dim, 0.16);
            });
            output.textures_delta.clear();
            assert_eq!(opacity, if dim { 1.0 } else { 0.0 });
        }
    }

    #[test]
    fn a_level_chip_holds_then_fades_out() {
        let level = Level {
            name: "Font size",
            value: "15 pt".into(),
            shown: 0.0,
        };
        assert_eq!(level.opacity(0.0), Some(1.0));
        assert_eq!(level.opacity(Level::HOLD), Some(1.0));
        let fading = level.opacity(Level::HOLD + Level::FADE * 0.5).unwrap();
        assert!(
            fading > 0.0 && fading < 0.5,
            "ease-out fades early: {fading}"
        );
        assert_eq!(level.opacity(Level::HOLD + Level::FADE), None);
    }

    fn palette() -> Palette {
        Palette::new(Appearance::Dark, Accent::default())
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette());
        ctx
    }

    fn frame(ctx: &egui::Context, events: Vec<Event>, mut add: impl FnMut(&mut Ui)) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 320.0))),
                events,
                ..Default::default()
            },
            |ui| add(ui),
        );
        output.textures_delta.clear();
    }

    fn click(ctx: &egui::Context, pos: Pos2, mut add: impl FnMut(&mut Ui)) {
        frame(ctx, vec![Event::PointerMoved(pos)], &mut add);
        for pressed in [true, false] {
            frame(
                ctx,
                vec![Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }],
                &mut add,
            );
        }
    }

    #[test]
    fn toggle_flips_once_per_click_and_keeps_its_identity() {
        let ctx = context();
        let p = palette();
        let mut on = false;
        let mut target = None;
        let mut id = None;
        frame(&ctx, vec![], |ui| {
            let response = toggle(ui, p, &mut on, "Blink cursor");
            target = Some(response.rect.center());
            id = Some(response.id);
        });
        let mut after = None;
        click(&ctx, target.unwrap(), |ui| {
            after = Some(toggle(ui, p, &mut on, "Blink cursor").id);
        });
        assert!(on, "one click turns the switch on");
        assert_eq!(id, after);
        frame(&ctx, vec![], |ui| {
            toggle(ui, p, &mut on, "Blink cursor");
        });
        assert!(on, "an idle frame does not toggle again");
    }

    #[test]
    fn segmented_selects_the_clicked_option_only() {
        let ctx = context();
        let p = palette();
        let options = [(0u8, "Block"), (1, "Beam"), (2, "Underline")];
        let mut value = 0u8;
        let mut origin = Pos2::ZERO;
        frame(&ctx, vec![], |ui| {
            origin = ui.cursor().min;
            segmented(ui, p, "cursor", &mut value, &options, 240.0);
        });
        // The third of three equal segments.
        click(&ctx, origin + vec2(200.0, 14.0), |ui| {
            segmented(ui, p, "cursor", &mut value, &options, 240.0);
        });
        assert_eq!(value, 2);
    }

    #[test]
    fn a_segmented_label_lights_under_the_pointer_and_its_cell_rings_with_the_keyboard() {
        // Terminal's segmented control has no fill of its own under the
        // pointer: the label there turns from secondary to fg. A cell with
        // the keyboard is ringed, and once the thumb has slid, the control
        // rests.
        let ctx = context();
        let p = palette();
        let options = [(0u8, "Block"), (1, "Beam"), (2, "Underline")];
        let mut value = 0u8;
        // One frame: what it painted, how soon it asked for another, and
        // where the control began.
        let mut run = |events: Vec<Event>, focus: bool| {
            let mut origin = Pos2::ZERO;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 320.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if focus {
                        let id = ui.id().with(("segmented", "cursor")).with("Beam");
                        ui.memory_mut(|memory| memory.request_focus(id));
                    }
                    origin = ui.cursor().min;
                    segmented(ui, p, "cursor", &mut value, &options, 240.0);
                },
            );
            output.textures_delta.clear();
            let wait = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            let shapes: Vec<egui::Shape> = (output.shapes.into_iter())
                .map(|clipped| clipped.shape)
                .collect();
            (shapes, wait, origin)
        };
        let ink = |shapes: &[egui::Shape], text: &str| {
            shapes.iter().find_map(|shape| match shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.fallback_color)
                }
                _ => None,
            })
        };
        let (shapes, _, origin) = run(Vec::new(), false);
        assert_eq!(ink(&shapes, "Beam"), Some(p.secondary), "at rest");
        assert_eq!(ink(&shapes, "Block"), Some(p.fg), "chosen");
        // The middle of three equal cells.
        let beam = origin + vec2(120.0, 14.0);
        let (shapes, ..) = run(vec![Event::PointerMoved(beam)], false);
        assert_eq!(ink(&shapes, "Beam"), Some(p.fg), "under the pointer");
        run(vec![Event::PointerGone], true);
        let (shapes, ..) = run(Vec::new(), true);
        let ring = theme::tint(p.accent, 0.55);
        let ringed = (shapes.iter()).any(|shape| {
            matches!(shape, egui::Shape::Rect(rect)
                if rect.stroke.color == ring && rect.rect.contains(beam))
        });
        assert!(ringed, "the cell with the keyboard is ringed");
        let mut wait = std::time::Duration::ZERO;
        for _ in 0..30 {
            wait = run(Vec::new(), true).1;
        }
        assert_eq!(wait, std::time::Duration::MAX, "it rests");
    }

    #[test]
    fn segmented_labels_keep_room_on_both_sides() {
        // Neptune Video, spec §5.4 and P9-V4: equal cells at Medium 12 where
        // the labels fit; "Ask | Command" at 136 gives Command the room
        // its longer label needs instead of crowding it.
        let ctx = context();
        let label = |ui: &Ui, text: &str, size| {
            (ui.painter())
                .layout_no_wrap(text.to_owned(), theme::medium(size), Color32::PLACEHOLDER)
                .size()
                .x
        };
        frame(&ctx, vec![], |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(228.0, 28.0));
            let options = [(0u8, "Light"), (1, "Dark"), (2, "System")];
            let (cells, size) = segment_cells(ui, rect, &options);
            let cells: Vec<Rect> = cells.collect();
            assert_eq!(size, 12.0);
            assert!(
                cells
                    .iter()
                    .all(|cell| (cell.width() - 224.0 / 3.0).abs() < 0.01)
            );

            let rect = Rect::from_min_size(Pos2::ZERO, vec2(136.0, 28.0));
            let options = [(0u8, "Ask"), (1, "Command")];
            let (cells, size) = segment_cells(ui, rect, &options);
            let cells: Vec<Rect> = cells.collect();
            assert_eq!(size, 11.5);
            assert!(
                (cells[0].left() - 2.0).abs() < 0.01 && (cells[1].right() - 134.0).abs() < 0.01
            );
            assert!((cells[0].right() - cells[1].left()).abs() < 0.01);
            let room = (cells[1].width() - label(ui, "Command", 11.5)) / 2.0;
            assert!(room >= SEGMENT_ROOM, "Command has {room} on each side");
            let other = (cells[0].width() - label(ui, "Ask", 11.5)) / 2.0;
            assert!((room - other).abs() < 0.01, "the same room on both");

            // At its fit, each label keeps exactly its room.
            let fit = segmented_fit(ui, &["Ask", "Command"]);
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(fit, 28.0));
            let (cells, size) = segment_cells(ui, rect, &options);
            let cells: Vec<Rect> = cells.collect();
            assert_eq!(size, 11.5);
            let room = (cells[1].width() - label(ui, "Command", 11.5)) / 2.0;
            assert!((room - SEGMENT_ROOM).abs() < 0.01, "Command has {room}");

            // P10: the Speed card's rates at 760 stay equal, as the section
            // switch over them does, while each keeps its snug room.
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(233.0, 28.0));
            let options = [(0u8, "0.25×"), (1, "0.5×"), (2, "1×"), (3, "2×"), (4, "4×")];
            let (cells, size) = segment_cells(ui, rect, &options);
            let cells: Vec<Rect> = cells.collect();
            assert_eq!(size, 11.5);
            assert!(
                cells
                    .iter()
                    .all(|cell| (cell.width() - 229.0 / 5.0).abs() < 0.01),
                "{cells:?}"
            );
            let room = (cells[0].width() - label(ui, "0.25×", 11.5)) / 2.0;
            assert!(
                (SEGMENT_SNUG..SEGMENT_ROOM).contains(&room),
                "0.25× has {room}"
            );
        });
    }

    #[test]
    fn slider_snaps_to_steps_and_clamps_to_its_range() {
        let ctx = context();
        let p = palette();
        let mut value = 1.4f32;
        let mut rect = Rect::NOTHING;
        frame(&ctx, vec![], |ui| {
            rect = slider(ui, p, "Line spacing", &mut value, 1.0..=2.0, 0.05, 200.0).rect;
        });
        click(
            &ctx,
            Pos2::new(rect.right() + 40.0, rect.center().y),
            |ui| {
                slider(ui, p, "Line spacing", &mut value, 1.0..=2.0, 0.05, 200.0);
            },
        );
        // A press outside the track is not this slider's interaction.
        assert_eq!(value, 1.4);
        click(&ctx, Pos2::new(rect.right() - 1.0, rect.center().y), |ui| {
            slider(ui, p, "Line spacing", &mut value, 1.0..=2.0, 0.05, 200.0);
        });
        assert_eq!(value, 2.0);
        click(&ctx, rect.center(), |ui| {
            slider(ui, p, "Line spacing", &mut value, 1.0..=2.0, 0.05, 200.0);
        });
        assert!((value - 1.5).abs() < 0.001, "snapped to a step: {value}");
    }

    #[test]
    fn stepper_stops_at_its_bounds() {
        let ctx = context();
        let p = palette();
        let mut value = 31.0f32;
        let mut origin = Pos2::ZERO;
        frame(&ctx, vec![], |ui| {
            origin = ui.cursor().min;
            stepper(ui, p, "Font size", &mut value, 9.0..=32.0, 1.0, "31");
        });
        let plus = origin + vec2(112.0 - 15.0, 14.0);
        for _ in 0..3 {
            click(&ctx, plus, |ui| {
                stepper(ui, p, "Font size", &mut value, 9.0..=32.0, 1.0, "");
            });
        }
        assert_eq!(value, 32.0);
    }

    #[test]
    fn chords_split_into_keys_and_keep_a_literal_plus() {
        assert_eq!(chord_keys("Ctrl+Shift+D"), ["Ctrl", "Shift", "D"]);
        assert_eq!(chord_keys("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(chord_keys("Ctrl+-"), ["Ctrl", "-"]);
        assert_eq!(chord_keys("+"), ["+"]);
        assert_eq!(chord_keys("⌘D"), ["⌘D"]);
        assert_eq!(chord_keys("Esc"), ["Esc"]);
    }

    #[test]
    fn under_reduced_motion_an_eased_change_lands_at_once() {
        let ctx = egui::Context::default();
        let mut at = Vec::new();
        for (time, reduce) in [(0.0, false), (0.016, false), (0.032, true), (0.048, true)] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    kit::set_reduced_motion(ui.ctx(), reduce);
                    // Off at first, then asked to come on.
                    let on = time > 0.0;
                    at.push(animate(ui.ctx(), Id::new("hover"), on, 0.12));
                    at.push(animate(ui.ctx(), Id::new("press"), time > 0.02, 0.12));
                },
            );
            output.textures_delta.clear();
        }
        // Eased while motion is allowed: part way after one frame.
        assert!(at[2] > 0.0 && at[2] < 1.0, "{at:?}");
        // With Reduce motion on, the next change is whole on its first frame.
        assert_eq!(at[5], 1.0, "{at:?}");
        assert_eq!(at[4], 1.0, "{at:?}");
    }
}
