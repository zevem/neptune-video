//! Components the editor's areas share that Neptune Terminal has no
//! counterpart for: the wells the viewer and timeline sit in, the number
//! field, the keyframe controls of a row, the select list, chips, badges,
//! progress and the quiet statement an empty region makes. Each is drawn from
//! the same tokens and primitives as [`super::controls`], so it reads as one
//! of them. Terminal's spinner and rich tooltip, which it keeps among its
//! helpers rather than its controls, are ported here too.

use super::controls::{self, ButtonKind, elided, focus_ring, galley_at};
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Id, Layout, Painter, Pos2, Rect,
    Response, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType, pos2,
    vec2,
};
use std::ops::RangeInclusive;

// ---- Reduce motion ------------------------------------------------------

fn reduce_motion_id() -> Id {
    Id::new("reduce-motion")
}

/// Whether the person asked for eased changes to land at once.
pub fn reduced_motion(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(reduce_motion_id()))
        .unwrap_or(false)
}

/// Records the Reduce motion preference for every control to read. The app
/// sets it each frame from its settings.
pub fn set_reduced_motion(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|data| data.insert_temp(reduce_motion_id(), on));
}

// ---- Wells, corner caps and scroll fades --------------------------------

/// A pane's well: `fill` in the pane's corner, with a hairline inside its
/// edge.
pub fn pane_well(painter: &Painter, rect: Rect, fill: Color32, p: &Palette) {
    painter.rect_filled(rect, metrics::PANE_RADIUS, fill);
    painter.rect_stroke(rect, metrics::PANE_RADIUS, p.hairline(), StrokeKind::Inside);
}

/// Rounds a pane's corners over whatever was drawn into them: an open
/// quarter arc in the window material at each corner, wide enough to cover
/// the square corner a picture or a lane leaves. Paint it last, on the pane's
/// top layer.
pub fn corner_caps(painter: &Painter, rect: Rect, ground: Color32) {
    let radius = f32::from(metrics::PANE_RADIUS);
    let inset = rect.shrink(radius);
    // Each corner's arc centre, and the angle its quarter starts at.
    let corners = [
        (inset.left_top(), 180.0_f32),
        (inset.right_top(), 270.0),
        (inset.right_bottom(), 0.0),
        (inset.left_bottom(), 90.0),
    ];
    for (centre, from) in corners {
        let points = (0..8)
            .map(|step| {
                let angle = (from + 90.0 * step as f32 / 7.0).to_radians();
                centre + vec2(angle.cos(), angle.sin()) * (radius + 3.0)
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(6.0, ground)));
    }
}

/// The edge of a scrolled region that content runs on past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

/// Says that content runs on past `edge` of `rect`: two flat 6-point bands of
/// the ground the region sits on, the outer the denser. Show it only while
/// something is clipped there.
pub fn scroll_fade(painter: &Painter, rect: Rect, edge: Edge, ground: Color32) {
    for (offset, alpha) in [(0.0, 0.6), (6.0, 0.3)] {
        let band = match edge {
            Edge::Top => Rect::from_min_size(
                pos2(rect.left(), rect.top() + offset),
                vec2(rect.width(), 6.0),
            ),
            Edge::Bottom => Rect::from_min_size(
                pos2(rect.left(), rect.bottom() - offset - 6.0),
                vec2(rect.width(), 6.0),
            ),
        };
        painter.rect_filled(band, 0, theme::tint(ground, alpha));
    }
}

/// The scroll fades of what a scroll area shows: one into `ground` at each
/// edge its content runs on past, so that a line cut there is read as more
/// and not as a sliver. None at an edge with nothing beyond it.
pub fn scroll_fades<R>(ui: &Ui, output: &egui::scroll_area::ScrollAreaOutput<R>, ground: Color32) {
    let shown = output.inner_rect;
    let above = output.state.offset.y;
    let below = output.content_size.y - above - shown.height();
    let painter = ui.painter_at(shown);
    if above > 0.5 {
        scroll_fade(&painter, shown, Edge::Top, ground);
    }
    if below > 0.5 {
        scroll_fade(&painter, shown, Edge::Bottom, ground);
    }
}

// ---- Icon buttons --------------------------------------------------------

/// Splits a control's name into what it does and the shortcut it gives in
/// a trailing "(…)", so that the tooltip can set the keys as keycaps. Words
/// in brackets that are not keys, such as a count, stay in the name.
pub fn name_and_keys(label: &str) -> (&str, &str) {
    label
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .filter(|(_, keys)| is_chord(keys))
        .unwrap_or((label, ""))
}

/// Whether `text` is a shortcut as the interface writes one: keys joined
/// by "+", or led by the macOS modifier marks, with no spaces.
fn is_chord(text: &str) -> bool {
    const NAMED: [&str; 20] = [
        "Ctrl",
        "Cmd",
        "Shift",
        "Alt",
        "Option",
        "Super",
        "Enter",
        "Return",
        "Space",
        "Tab",
        "Esc",
        "Escape",
        "Delete",
        "Backspace",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Insert",
        "Fn",
    ];
    let key = |key: &str| {
        let key = key.trim_start_matches(['⌘', '⇧', '⌥', '⌃']);
        key.is_empty()
            || key.chars().count() == 1
            || NAMED.contains(&key)
            || key
                .strip_prefix('F')
                .and_then(|number| number.parse::<u8>().ok())
                .is_some_and(|number| (1..=24).contains(&number))
    };
    !text.is_empty()
        && !text.contains(char::is_whitespace)
        && controls::chord_keys(text).into_iter().all(key)
}

/// How far a control's hover has come in, from nought to one: eased over
/// 0.12 s, and at once under Reduce motion. A control that cannot be used is
/// never hovered.
// Neptune Video: Terminal lights a hover at once; the spec eases it.
pub fn hover_amount(ui: &Ui, response: &Response) -> f32 {
    controls::animate(
        ui.ctx(),
        response.id.with("hover"),
        ui.is_enabled() && response.hovered(),
        theme::motion::HOVER,
    )
}

/// The look of an icon button: Terminal's surface, ring and glyph over a
/// response the caller made. `on` holds the surface pressed, as a toggle
/// that is on does; `tint` recolours the glyph for a state that is not on or
/// off, such as record. Every state is drawn from `p`, so a button on the
/// stage takes the stage's palette in either appearance.
pub fn paint_icon_button(
    ui: &Ui,
    p: &Palette,
    response: &Response,
    icon: Icon,
    on: bool,
    tint: Option<Color32>,
) {
    let rect = response.rect;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let surface = rect.shrink(1.0);
    if !ui.is_enabled() {
        if on {
            painter.rect_filled(surface, 7, p.pressed);
        }
        icons::paint(painter, rect.shrink(6.0), icon, tint.unwrap_or(p.muted));
        return;
    }
    let down = response.is_pointer_button_down_on();
    let hover = hover_amount(ui, response);
    if on || down {
        painter.rect_filled(surface, 7, p.pressed);
    } else if hover > 0.0 {
        painter.rect_filled(surface, 7, p.hover.gamma_multiply(hover));
    }
    if response.has_focus() {
        painter.rect_stroke(surface, 7, Stroke::new(1.5, p.accent), StrokeKind::Inside);
    }
    // The glyph rests quieter than the label ink and comes up to it as the
    // pointer arrives, as Terminal's icon buttons do.
    let ink = match tint {
        Some(tint) => tint,
        None if on || down || response.has_focus() => p.fg,
        None => theme::mix(p.secondary, p.fg, hover),
    };
    // Pressing settles the glyph slightly, like a physical key.
    let inset = if down { 6.5 } else { 6.0 };
    icons::paint(painter, rect.shrink(inset), icon, ink);
}

/// The look of a small icon button: a 22-point target whose whole surface
/// lights, around a 14-point glyph. `on` holds it pressed.
pub fn paint_small_icon_button(ui: &Ui, p: &Palette, response: &Response, icon: Icon, on: bool) {
    let rect = response.rect;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let enabled = ui.is_enabled();
    let hover = hover_amount(ui, response);
    if on || (enabled && response.is_pointer_button_down_on()) {
        painter.rect_filled(rect, 6, p.pressed);
    } else if hover > 0.0 {
        painter.rect_filled(rect, 6, p.hover.gamma_multiply(hover));
    }
    if response.has_focus() {
        focus_ring(painter, rect, 6, *p);
    }
    let ink = if !enabled {
        p.muted
    } else if on || response.has_focus() {
        p.fg
    } else {
        theme::mix(p.secondary, p.fg, hover)
    };
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        icon,
        ink,
    );
}

/// An icon-only control for the inside of a row, named by its tooltip.
pub fn small_icon_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    let (_, response) = ui.allocate_exact_size(Vec2::splat(metrics::SMALL_BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    paint_small_icon_button(ui, p, &response, icon, false);
    let (name, keys) = name_and_keys(label);
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        name,
        keys,
    )
}

/// As [`small_icon_button`], for a state: pressed while it is on.
pub fn small_icon_toggle(ui: &mut Ui, p: &Palette, icon: Icon, on: bool, label: &str) -> Response {
    let (_, response) = ui.allocate_exact_size(Vec2::splat(metrics::SMALL_BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    paint_small_icon_button(ui, p, &response, icon, on);
    let (name, keys) = name_and_keys(label);
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        name,
        keys,
    )
}

// ---- Rows ----------------------------------------------------------------

/// Where a labelled row puts its parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowFit {
    pub height: f32,
    /// Room before the label.
    pub lead: f32,
    /// Room after the controls.
    pub trail: f32,
}

impl RowFit {
    /// A row of an inspector card.
    pub const INSPECTOR: Self = Self {
        height: metrics::INSPECTOR_ROW,
        lead: 12.0,
        trail: 8.0,
    };
    /// A row of an inspector card that also holds keyed rows: its controls
    /// stop where the keyed rows' fields do, clear of the key column.
    pub const INSPECTOR_BESIDE_KEYS: Self = Self {
        height: metrics::INSPECTOR_ROW,
        lead: 12.0,
        trail: 8.0 + metrics::KEY_COLUMN + 8.0,
    };
}

fn row_label(text: &str, p: &Palette) -> egui::RichText {
    egui::RichText::new(text)
        .font(theme::regular(12.5))
        .color(p.fg)
}

/// A label at the leading edge and the controls `add` makes at the
/// trailing edge of one row.
pub fn row<R>(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    fit: RowFit,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(fit.height);
        if fit.lead > 0.0 {
            ui.add_space(fit.lead);
        }
        ui.label(row_label(label, p));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if fit.trail > 0.0 {
                ui.add_space(fit.trail);
            }
            add(ui)
        })
        .inner
    })
    .inner
}

/// As [`row`], with a label that is cut with an ellipsis rather than pushed
/// under its controls when the panel is narrow.
pub fn fitted_row<R>(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    fit: RowFit,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(fit.height);
        if fit.lead > 0.0 {
            ui.add_space(fit.lead);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if fit.trail > 0.0 {
                ui.add_space(fit.trail);
            }
            let inner = add(ui);
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(row_label(label, p)).truncate());
            });
            inner
        })
        .inner
    })
    .inner
}

// ---- Number field --------------------------------------------------------

/// How a number field reads and steps its value.
#[derive(Clone, Copy, Debug)]
pub struct Number<'a> {
    pub range: &'a RangeInclusive<f32>,
    pub speed: f64,
    /// The axis, set before the value where the row's label says it: "X",
    /// "Y", "W", "H".
    pub prefix: Option<&'a str>,
    /// The unit, as written after the value: " %", " s", "°".
    pub suffix: &'a str,
    pub decimals: usize,
}

/// A number that is dragged or typed, in a 26-point field that takes `size`.
/// It is egui's drag value inside the surface, so its identity, typing and
/// scrubbing are egui's; `label` names it beside its row. The axis is
/// painted quieter than the value, which stands right-aligned before the
/// unit; the unit is in the value's ink, as Terminal's fields write it.
pub fn number_field(
    ui: &mut Ui,
    p: &Palette,
    value: &mut f32,
    number: Number,
    size: Vec2,
    label: Option<&str>,
) -> Response {
    let surface = ui.painter().add(Shape::Noop);
    let unit_font = theme::regular(12.5);
    let unit = ui.painter().layout_no_wrap(
        number.suffix.to_owned(),
        unit_font.clone(),
        if ui.is_enabled() { p.fg } else { p.muted },
    );
    let enabled = ui.is_enabled();
    let response = ui
        .scope(|ui| {
            // The drag value draws no surface of its own: this one is drawn
            // beneath it.
            let style = ui.style_mut();
            style
                .text_styles
                .insert(style.drag_value_text_style.clone(), theme::regular(12.5));
            // The field is as large as it is asked to be, and the value
            // ends 8 points in from the unit.
            style.spacing.interact_size = size;
            style.spacing.button_padding = vec2(8.0, 0.0);
            let visuals = &mut style.visuals;
            if !enabled {
                visuals.override_text_color = Some(p.muted);
            }
            visuals.text_edit_bg_color = Some(Color32::TRANSPARENT);
            visuals.selection.stroke = Stroke::NONE;
            for widget in [
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                widget.weak_bg_fill = Color32::TRANSPARENT;
                widget.bg_fill = Color32::TRANSPARENT;
                widget.bg_stroke = Stroke::NONE;
                widget.corner_radius = CornerRadius::same(7);
            }
            // The unit is laid out unseen after the value, which keeps the
            // value clear of it and its text in the value read out; it is
            // painted over that place below.
            let drag = egui::DragValue::new(value)
                .range(number.range.clone())
                .speed(number.speed)
                .suffix(
                    egui::RichText::new(number.suffix)
                        .font(unit_font.clone())
                        .color(Color32::TRANSPARENT),
                )
                .fixed_decimals(number.decimals);
            let layout =
                Layout::centered_and_justified(ui.layout().main_dir()).with_main_align(Align::Max);
            ui.allocate_ui_with_layout(size, layout, |ui| ui.add(drag))
                .inner
        })
        .inner;
    if let Some(label) = label {
        response.widget_info(|| WidgetInfo::labeled(WidgetType::DragValue, enabled, label));
    }
    let rect = Rect::from_center_size(
        response.rect.center(),
        vec2(
            response.rect.width(),
            metrics::INSPECTOR_CONTROL.min(response.rect.height()),
        ),
    );
    let mut shapes = vec![Shape::rect_filled(rect, 7, p.control)];
    let editing = response.has_focus();
    if enabled && response.hovered() && !editing {
        shapes.push(Shape::rect_filled(rect, 7, p.hover));
    }
    if editing {
        // A field being typed in: an accent outline with a soft halo.
        shapes.push(Shape::rect_stroke(
            rect.expand(2.5),
            9,
            Stroke::new(3.0, theme::tint(p.accent, 0.28)),
            StrokeKind::Inside,
        ));
        shapes.push(Shape::rect_stroke(
            rect,
            7,
            Stroke::new(1.0, p.accent),
            StrokeKind::Inside,
        ));
    }
    ui.painter().set(surface, Shape::Vec(shapes));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if let Some(prefix) = number.prefix {
            // The axis is all that tells twin fields apart, so it is set in
            // `secondary`, which reads on the field, rather than `muted`.
            let axis = painter.layout_no_wrap(prefix.to_owned(), theme::medium(11.0), p.secondary);
            galley_at(painter, pos2(rect.left() + 8.0, rect.center().y), axis);
        }
        // While it is typed in, the field holds only the number.
        if !editing && !number.suffix.is_empty() {
            let at = pos2(rect.right() - 8.0 - unit.size().x, rect.center().y);
            galley_at(painter, at, unit);
        }
    }
    response
}

// ---- Keyframe controls ---------------------------------------------------

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

/// The keyframe diamond: an outline while the value is not animated, ink
/// once it is, and filled while the playhead sits on a keyframe. It is never
/// the accent: an animated value is a state, not a selection.
pub fn key_toggle(ui: &mut Ui, p: &Palette, state: KeyState, label: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::SMALL_BUTTON), Sense::click());
    let animated = state != KeyState::Off;
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), animated, label)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let enabled = ui.is_enabled();
        if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(rect, 6, p.pressed);
        } else if enabled && response.hovered() {
            painter.rect_filled(rect, 6, p.hover);
        }
        if response.has_focus() {
            focus_ring(painter, rect, 6, *p);
        }
        let ink = if !enabled {
            p.muted
        } else if animated || response.hovered() {
            p.fg
        } else {
            // Quiet, but as plain as a control that can be pressed must be:
            // about 3:1 on a card in either appearance.
            p.muted
        };
        let on_key = matches!(state, KeyState::Animated { on_key: true, .. });
        icons::paint(
            painter,
            Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
            if on_key {
                Icon::DiamondFilled
            } else {
                Icon::Diamond
            },
            ink,
        );
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// A step to the keyframe on one side of the playhead: a chevron in the
/// 18-point slot beside the diamond.
fn key_step(ui: &mut Ui, p: &Palette, icon: Icon, enabled: bool, label: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if enabled && response.is_pointer_button_down_on() {
                painter.rect_filled(rect, 6, p.pressed);
            } else if enabled && response.hovered() {
                painter.rect_filled(rect, 6, p.hover);
            }
            if response.has_focus() {
                focus_ring(painter, rect, 6, *p);
            }
            let ink = if !enabled {
                p.muted
            } else if response.hovered() {
                p.fg
            } else {
                p.secondary
            };
            icons::paint(
                painter,
                Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
                icon,
                ink,
            );
        }
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(label)
    })
    .inner
}

/// The key column at the end of a keyed row: the step back, the diamond and
/// the step on, in `column`. The steps' slots are kept on a row keyframes do
/// not drive, so the diamond stands at one place on every row. `id` is the
/// row's, which keeps the controls theirs as the row moves.
pub fn key_cluster(
    ui: &mut Ui,
    p: &Palette,
    id: Id,
    column: Rect,
    what: &str,
    state: KeyState,
) -> KeyRow {
    let slot = |left: f32, width: f32| {
        Rect::from_min_size(
            pos2(column.left() + left, column.center().y - 11.0),
            vec2(width, 22.0),
        )
    };
    let (previous, next) = match state {
        KeyState::Off => (None, None),
        KeyState::Animated { previous, next, .. } => (Some(previous), Some(next)),
    };
    // Each control is the row's by name, not by how many came before it:
    // the diamond keeps the keyboard when the steps come in beside it.
    let mut place = |rect: Rect, part: &'static str, add: &mut dyn FnMut(&mut Ui) -> Response| {
        let mut child = ui.new_child(
            UiBuilder::new()
                .id(id.with(("key-cluster", part)))
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        add(&mut child)
    };
    let previous = previous.map(|there| {
        let label = format!("Previous {what} keyframe");
        place(slot(0.0, 18.0), "previous", &mut |ui| {
            key_step(ui, p, Icon::ChevronLeft, there, &label)
        })
    });
    let label = match state {
        KeyState::Off => format!("Animate {what}"),
        KeyState::Animated { on_key: true, .. } => format!("Remove {what} keyframe"),
        KeyState::Animated { .. } => format!("Add {what} keyframe"),
    };
    let toggle = place(slot(20.0, 22.0), "diamond", &mut |ui| {
        key_toggle(ui, p, state, &label)
    });
    let next = next.map(|there| {
        let label = format!("Next {what} keyframe");
        place(slot(44.0, 18.0), "next", &mut |ui| {
            key_step(ui, p, Icon::ChevronRight, there, &label)
        })
    });
    KeyRow {
        toggle,
        previous,
        next,
    }
}

/// A row whose value can be keyframed: the label at the leading edge, the
/// controls `lead` and `add` make at the trailing edge, and after them the
/// key column, which stands at the same place on every row. The label is
/// never cut: where the row has no room for it, it takes a line of its own
/// above the controls. `what` names the value for the key controls.
#[expect(clippy::too_many_arguments, reason = "a row is all of these")]
pub fn keyed_row(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    what: &str,
    state: KeyState,
    fit: RowFit,
    add: impl FnOnce(&mut Ui),
    lead: impl FnOnce(&mut Ui),
) -> KeyRow {
    // The row is made in reading order, which is the order Tab walks it in:
    // the label, then the fields, then the steps and the diamond. How wide
    // the fields are is known only once they are made: it is kept from the
    // frame before, and a row whose controls changed width is laid out again
    // before it is shown.
    let id = ui.next_auto_id().with("keyed-row");
    // What the controls took the frame before: their width and height, and
    // how far below the middle of their place they stood. A field egui grows
    // past its room grows downward, and the row lifts it back to the middle,
    // where the label and the key column are.
    let known: Option<(Vec2, f32)> = ui.data(|data| data.get_temp(id));
    let controls = known.map_or(86.0, |(known, _)| known.x);
    let tall = known.map_or(fit.height, |(known, _)| known.y.max(fit.height));
    let lift = known.map_or(0.0, |(_, below)| below);
    let width = ui.available_width();
    let text = row_label(label, p);
    let whole = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::regular(12.5), p.fg)
        .size()
        .x;
    let wraps = fit.lead + whole + 8.0 + controls + 8.0 + metrics::KEY_COLUMN + fit.trail > width;
    let height = tall + if wraps { 20.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    // The whole row is one child of a fixed identity, so what comes after it
    // keeps its own whether or not this row shows its steps.
    let mut ui = ui.new_child(
        UiBuilder::new()
            .id(id.with("row"))
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let ui = &mut ui;
    let line = Rect::from_min_max(pos2(rect.left(), rect.bottom() - tall), rect.max);
    let label_line = if wraps {
        Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 20.0))
    } else {
        line
    };
    controls::place(
        ui,
        Rect::from_min_max(
            pos2(label_line.left() + fit.lead, label_line.top()),
            label_line.max,
        ),
        Layout::left_to_right(Align::Center),
        (id, "label"),
        |ui| ui.add(egui::Label::new(text).extend()),
    );
    let column = Rect::from_min_max(
        pos2(line.right() - fit.trail - metrics::KEY_COLUMN, line.top()),
        pos2(line.right() - fit.trail, line.bottom()),
    );
    let fields = Rect::from_min_max(
        pos2(
            (column.left() - 8.0 - controls).max(line.left()),
            line.top(),
        ),
        pos2(column.left() - 8.0, line.bottom()),
    )
    .translate(vec2(0.0, -lift));
    let mut row = ui.new_child(
        UiBuilder::new()
            .id_salt((id, "controls"))
            .max_rect(fields)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let from = row.cursor().left();
    lead(&mut row);
    // The fields are named by the row and by nothing made before them in
    // it. A row gains and loses its reset and its steps while a value is
    // typed, and a field named by its place among them would be another
    // field from then on: it would lose the keyboard part way through.
    row.scope_builder(UiBuilder::new().id(id.with("fields")), add);
    let used = vec2(row.min_rect().right() - from, row.min_rect().height());
    let below = row.min_rect().center().y - fields.center().y;
    let moved = |(known, lifted): (Vec2, f32)| {
        (known - used).abs().max_elem() > 0.5 || (lifted - below).abs() > 0.5
    };
    if known.is_none_or(moved) {
        ui.data_mut(|data| data.insert_temp(id, (used, below)));
        ui.ctx()
            .request_discard("a keyed row's controls changed size");
    }
    key_cluster(ui, p, id, column, what, state)
}

// ---- Select list ---------------------------------------------------------

/// A field showing the current choice that opens a list of the others,
/// `height` tall: Terminal's select, 30 in a sheet and 26 in an inspector
/// row. The caller shows the list with [`select_list`] or its own menu.
pub fn select(ui: &mut Ui, p: &Palette, id: Id, value: &str, label: &str, size: Vec2) -> Response {
    let id = controls::viewport_id(ui.ctx(), id);
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, ui.is_enabled(), label));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_value(value);
    });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let radius = metrics::CONTROL_RADIUS;
    let painter = ui.painter();
    if ui.is_enabled() && (response.hovered() || open) {
        painter.rect_filled(rect, radius, p.pressed);
    }
    painter.rect_stroke(rect, radius, p.hairline(), StrokeKind::Inside);
    if response.has_focus() {
        focus_ring(painter, rect, radius, *p);
    }
    icons::paint(
        painter,
        Rect::from_center_size(
            Pos2::new(rect.right() - 16.0, rect.center().y),
            Vec2::splat(12.0),
        ),
        Icon::ChevronDown,
        p.secondary,
    );
    let font = theme::regular(if size.y < metrics::CONTROL_HEIGHT {
        12.5
    } else {
        13.0
    });
    galley_at(
        painter,
        Pos2::new(rect.left() + 10.0, rect.center().y),
        elided(painter, value, font, p.fg, rect.width() - 38.0),
    );
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A select over named choices: the field, and its list of menu rows with a
/// check on the one chosen. Returns the field, marked changed when another
/// choice was taken.
pub fn select_list<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    id: Id,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
    size: Vec2,
) -> Response {
    let current = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or("", |(_, name)| *name);
    let mut response = select(ui, p, id, current, label, size);
    let mut chosen = None;
    egui::Popup::menu(&response).show(|ui| {
        controls::menu_layout(ui, size.x.max(240.0));
        let mut rows = |ui: &mut Ui| {
            for (option, name) in options {
                let row = menu_row(ui, p, None, name, "", false, Some(option == value));
                if row.clicked() {
                    chosen = Some(*option);
                }
            }
        };
        if options.len() > 9 {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, rows);
        } else {
            rows(ui);
        }
    });
    if let Some(option) = chosen
        && option != *value
    {
        *value = option;
        response.mark_changed();
    }
    response
}

/// A row of a menu or list: Terminal's menu row, which may have no icon, be
/// a choice with a check on the current one, or be disabled with its reason
/// in its tooltip.
pub fn menu_row(
    ui: &mut Ui,
    p: &Palette,
    icon: Option<Icon>,
    label: &str,
    shortcut: &str,
    destructive: bool,
    choice: Option<bool>,
) -> Response {
    // Named by its place, not its label: a list may hold one name twice.
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), metrics::COMPACT_HEIGHT),
        Sense::click(),
    );
    let enabled = ui.is_enabled();
    response.widget_info(|| match choice {
        Some(chosen) => WidgetInfo::selected(WidgetType::RadioButton, enabled, chosen, label),
        None => WidgetInfo::labeled(WidgetType::Button, enabled, label),
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    // The row the arrows are on is shown as the one the pointer is on.
    let highlighted = enabled && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    let (text, hint) = if highlighted {
        painter.rect_filled(rect, 6, if destructive { p.red } else { p.accent });
        let ink = if destructive { p.on_red() } else { p.on_accent };
        (ink, ink)
    } else if !enabled {
        // The disabled interface is already drawn at half strength.
        (p.muted, p.muted)
    } else if destructive {
        (p.red, p.muted)
    } else {
        (p.fg, p.muted)
    };
    let glyph = if choice == Some(true) {
        Some(Icon::Check)
    } else if choice.is_some() {
        None
    } else {
        icon
    };
    if let Some(glyph) = glyph {
        icons::paint(
            painter,
            Rect::from_center_size(
                Pos2::new(rect.left() + 15.0, rect.center().y),
                Vec2::splat(14.0),
            ),
            glyph,
            text,
        );
    }
    let keys = if shortcut.is_empty() {
        0.0
    } else {
        painter
            .layout_no_wrap(shortcut.into(), theme::regular(11.5), hint)
            .size()
            .x
            + 8.0
    };
    galley_at(
        painter,
        Pos2::new(rect.left() + 32.0, rect.center().y),
        elided(
            painter,
            label,
            theme::regular(13.0),
            text,
            rect.width() - 42.0 - keys,
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
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A heading inside a menu, aligned with the rows' labels and read out as
/// the label it is.
pub fn menu_heading(ui: &mut Ui, p: &Palette, text: &str) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 24.0));
    let words = ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::medium(11.5),
        p.secondary,
    );
    said(ui, words, text);
}

/// Words at the head of a menu, or where a heading would stand, that say
/// why the rows under them cannot be used: Regular 11.5 `secondary`, set
/// where the rows' words begin, wrapped within the menu and read as a
/// label.
pub fn menu_note(ui: &mut Ui, p: &Palette, text: &str) {
    let width = ui.available_width();
    let galley = ui.painter().layout(
        text.to_owned(),
        theme::regular(11.5),
        p.secondary,
        (width - 32.0 - 8.0).max(40.0),
    );
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y + 12.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    ui.painter()
        .galley(rect.min + vec2(32.0, 6.0), galley, Color32::PLACEHOLDER);
}

// ---- Chips, badges and progress -----------------------------------------

/// A pill that picks one of several named choices, such as a preset. The one
/// chosen is pressed, with a check before its name.
pub fn chip(ui: &mut Ui, p: &Palette, text: &str, selected: bool) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), theme::medium(12.0), Color32::PLACEHOLDER);
    let check = if selected { 16.0 } else { 0.0 };
    let size = vec2(10.0 + check + galley.size().x + 10.0, 26.0);
    let (_, response) = ui.allocate_exact_size(size, Sense::click());
    paint_chip(ui, p, &response, galley, selected);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Paints a chip over `response`, the chosen one with the check before its
/// name, so it is told from the others by more than its fill.
fn paint_chip(
    ui: &Ui,
    p: &Palette,
    response: &Response,
    galley: std::sync::Arc<egui::Galley>,
    selected: bool,
) {
    let rect = response.rect;
    let enabled = ui.is_enabled();
    let text = galley.text().to_owned();
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &text)
    });
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let hover = hover_amount(ui, response);
    painter.rect_filled(rect, 13, if selected { p.pressed } else { p.control });
    if enabled && response.is_pointer_button_down_on() {
        painter.rect_filled(rect, 13, p.pressed);
    } else if hover > 0.0 {
        painter.rect_filled(rect, 13, p.hover.gamma_multiply(hover));
    }
    if response.has_focus() {
        focus_ring(painter, rect, 13, *p);
    }
    let ink = if enabled { p.fg } else { p.muted };
    if selected {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(12.0)),
            Icon::Check,
            ink,
        );
        painter.galley(
            pos2(rect.left() + 26.0, rect.center().y - galley.size().y * 0.5),
            galley,
            ink,
        );
    } else {
        painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
    }
}

/// A small label over a picture, such as a duration: a dark band 16 tall
/// with white ink, its left edge at `left_center`. Numbers are set in the
/// monospace face. Returns the badge's rect.
pub fn badge(painter: &Painter, left_center: Pos2, text: &str) -> Rect {
    let numeric = text
        .chars()
        .all(|c| c.is_ascii_digit() || " .,:%×+-−".contains(c));
    let font = if numeric {
        theme::mono(11.0)
    } else {
        theme::medium(11.0)
    };
    let ink = Color32::WHITE;
    let galley = painter.layout_no_wrap(text.to_owned(), font, ink);
    let rect = Rect::from_min_size(
        pos2(left_center.x, left_center.y - 8.0),
        vec2(galley.size().x + 12.0, 16.0),
    );
    painter.rect_filled(rect, 8, Color32::from_black_alpha(140));
    painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
    rect
}

/// How far a piece of background work has got: a 4-point bar over the width
/// available, filled in `fill` (the accent while it runs, red once it has
/// failed), named `name` with its value for those who do not see it. Work
/// that has not said how far it has got is `None`: the bar is its bare
/// track, has no value to read out, and does not move.
pub fn progress_bar(
    ui: &mut Ui,
    p: &Palette,
    done: Option<f32>,
    fill: Color32,
    name: &str,
) -> Response {
    let done = done.map(|done| {
        if done.is_finite() {
            done.clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let (bar, response) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, true, name);
        info.value = done.map(f64::from);
        info
    });
    if ui.is_rect_visible(bar) {
        let painter = ui.painter();
        painter.rect_filled(bar, 2, p.control);
        if let Some(done) = done.filter(|done| *done > 0.0) {
            let mut filled = bar;
            // A started bar shows at least its rounded end.
            filled.set_width((bar.width() * done).max(4.0).min(bar.width()));
            painter.rect_filled(filled, 2, fill);
        }
    }
    response
}

/// A ring of how far work has got, 18 points across, as one path.
pub fn progress_ring(painter: &Painter, p: &Palette, center: Pos2, done: f32) {
    let radius = 9.0 - 1.25;
    painter.circle_stroke(center, radius, Stroke::new(2.5, theme::tint(p.fg, 0.14)));
    let done = if done.is_finite() {
        done.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if done <= 0.0 {
        return;
    }
    let steps = ((48.0 * done).ceil() as usize).max(2);
    let points = (0..=steps)
        .map(|step| {
            // From twelve o'clock, clockwise.
            let angle = -std::f32::consts::FRAC_PI_2
                + std::f32::consts::TAU * done * step as f32 / steps as f32;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(2.5, p.accent)));
}

// ---- Spinner and rich tooltip -------------------------------------------

/// Terminal's loading mark: the Tabler loader turning once a second, as
/// wide as `rect` allows. A mark out of view does not keep the window
/// repainting, and one in view repaints it at most 30 times a second.
pub fn spinner(ui: &Ui, rect: Rect, color: Color32) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    // Neptune Video: Terminal repaints every frame; the turn is read from
    // the clock, so a capped rate keeps its speed. egui takes what it
    // expects a pass to take off the wait, which is added back so the
    // window is woken a thirtieth of a second on, as `pacing` does.
    let predicted = ui.input(|input| input.predicted_dt);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f32(1.0 / 30.0 + predicted));
    let angle = (ui.input(|input| input.time) * std::f64::consts::TAU) as f32;
    // Tabler's 24-point cell has a 20-point ring, which is made as wide as
    // the rect less a point.
    let side = (rect.width().min(rect.height()) - 1.0).max(0.0) * 24.0 / 20.0;
    icons::paint_rotated(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(side)),
        Icon::Loader2,
        color,
        angle,
    );
}

/// A tooltip that says more than a name, as Terminal's pull request chip
/// does: a title, lines of detail under it, and a quiet hint of what a click
/// does, at most 300 points wide.
pub fn rich_tooltip(
    response: Response,
    p: &Palette,
    title: &str,
    lines: &[&str],
    hint: Option<&str>,
) -> Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(300.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        let label = |ui: &mut Ui, text: &str, font, color| {
            ui.add(
                egui::Label::new(egui::RichText::new(text).font(font).color(color))
                    .wrap()
                    .selectable(false),
            );
        };
        label(ui, title, theme::medium(12.5), p.fg);
        for line in lines {
            label(ui, line, theme::regular(11.5), p.secondary);
        }
        if let Some(hint) = hint {
            label(ui, hint, theme::regular(11.0), p.muted);
        }
    })
}

// ---- Empty state -----------------------------------------------------------

/// The quiet statement an empty region makes: a glyph, a title and a hint,
/// a little above the middle, with at most one action under them. It is
/// named for assistive technology as one statement. Returns whether its
/// action was pressed.
pub fn empty_state(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    icon: Icon,
    title: &str,
    hint: &str,
    action: Option<&str>,
) -> bool {
    let words = (icon, title, hint);
    statement(ui, p, rect, words, Some(28.0), action, |block| {
        rect.top() + rect.height() * 0.4 - block * 0.5
    })
}

/// The empty state of a small region, such as a plot: its glyph is 24, and
/// left out where the region is too short to hold it above the words with
/// room to spare. It offers no action.
pub fn small_empty_state(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    icon: Icon,
    title: &str,
    hint: &str,
) {
    const GLYPH: f32 = 24.0;
    // The room kept above and below the block.
    const ROOM: f32 = 16.0;
    let title_height = (ui.painter())
        .layout_no_wrap(title.to_owned(), theme::medium(14.0), p.fg)
        .size()
        .y;
    let glyph = (rect.height() >= GLYPH + 8.0 + title_height + ROOM * 2.0).then_some(GLYPH);
    statement(ui, p, rect, (icon, title, hint), glyph, None, |block| {
        rect.top() + rect.height() * 0.4 - block * 0.5
    });
}

/// An empty state whose block starts where `top` puts it, given its height.
/// The glyph is as large as `glyph` says, or left out.
fn statement(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    (icon, title, hint): (Icon, &str, &str),
    glyph: Option<f32>,
    action: Option<&str>,
    top: impl FnOnce(f32) -> f32,
) -> bool {
    let painter = ui.painter_at(rect);
    let width = (rect.width() - 32.0).clamp(40.0, 260.0);
    let mut job =
        egui::text::LayoutJob::simple(hint.to_owned(), theme::regular(12.0), p.secondary, width);
    job.halign = Align::Center;
    let hint_galley = painter.layout_job(job);
    let title_galley = painter.layout_no_wrap(title.to_owned(), theme::medium(14.0), p.fg);
    let action_height = if action.is_some() {
        12.0 + metrics::CONTROL_HEIGHT
    } else {
        0.0
    };
    let block = glyph.map_or(0.0, |size| size + 8.0)
        + title_galley.size().y
        + if hint.is_empty() {
            0.0
        } else {
            4.0 + hint_galley.size().y
        }
        + action_height;
    let top = top(block);
    let x = rect.center().x;
    let title_top = match glyph {
        Some(size) => {
            let glyph = Rect::from_center_size(pos2(x, top + size * 0.5), Vec2::splat(size));
            icons::paint(&painter, glyph, icon, p.muted);
            glyph.bottom() + 8.0
        }
        None => top,
    };
    let title_rect = Rect::from_min_size(
        pos2(x - title_galley.size().x * 0.5, title_top),
        title_galley.size(),
    );
    painter.galley(title_rect.min, title_galley, Color32::PLACEHOLDER);
    let mut bottom = title_rect.bottom();
    if !hint.is_empty() {
        let hint_top = bottom + 4.0;
        bottom = hint_top + hint_galley.size().y;
        painter.galley(pos2(x, hint_top), hint_galley, Color32::PLACEHOLDER);
    }
    // Painted text, named for assistive technology as one statement.
    let statement = if hint.is_empty() {
        title.to_owned()
    } else {
        format!("{title}. {hint}")
    };
    ui.interact(
        Rect::from_min_max(
            pos2(rect.left(), title_rect.top()),
            pos2(rect.right(), bottom),
        )
        .intersect(rect),
        ui.id().with(("empty-state", title)),
        Sense::hover(),
    )
    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &statement));
    let Some(label) = action else {
        return false;
    };
    let button = Rect::from_center_size(
        pos2(x, bottom + 12.0 + metrics::CONTROL_HEIGHT * 0.5),
        vec2(rect.width().min(240.0), metrics::CONTROL_HEIGHT),
    );
    controls::place(
        ui,
        button,
        Layout::top_down(Align::Center),
        ("empty-state-action", label),
        |ui| controls::button(ui, *p, label, ButtonKind::Secondary).clicked(),
    )
}

// ---- Painted words and sheets' keys -------------------------------------

/// Tells assistive tools of words that are only painted: `text` over
/// `rect`, where it is drawn, read as the label it stands for.
pub fn said(ui: &Ui, rect: Rect, text: &str) {
    ui.interact(rect, ui.auto_id_with(("said", text)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
}

/// Whether the sheet `id` is shown in this frame for the first time since it
/// was last away: the frame whose keys were meant for what was there before.
/// Asked again in the same frame, it says the same.
pub fn opening(ctx: &egui::Context, id: Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let key = id.with("shown");
    let seen = ctx.data(|data| data.get_temp::<(u64, u64)>(key));
    let first = match seen {
        Some((first, last)) if last == frame || last + 1 == frame => first,
        _ => frame,
    };
    ctx.data_mut(|data| data.insert_temp(key, (first, frame)));
    first == frame
}

/// The Enters of this frame: whether one was pressed afresh, and whether
/// one was held down or pressed as the sheet was `opening`, which was
/// meant for what was there before (spec §5.9.1).
pub fn enters(ctx: &egui::Context, opening: bool) -> (bool, bool) {
    ctx.input(|input| {
        (input.events.iter()).fold((false, false), |(fresh, carried), event| match event {
            egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                repeat,
                ..
            } => {
                let old = *repeat || opening;
                (fresh || !old, carried || old)
            }
            _ => (fresh, carried),
        })
    })
}

// ---- Splitter --------------------------------------------------------------

/// A gutter between two panes that is also their resize handle. Returns the
/// drag along its axis; a double click asks for the default size. Only its
/// grip is drawn: muted under the pointer, the accent while held.
pub fn splitter(ui: &mut Ui, p: &Palette, rect: Rect, id: Id, vertical: bool) -> (f32, bool) {
    // A gutter is the pointer's: it is not a place Tab stops at, where it
    // would hold the keyboard and show and do nothing.
    let response = ui.interact(rect.expand(1.0), id, Sense::CLICK | Sense::DRAG);
    let cursor = if vertical {
        CursorIcon::ResizeHorizontal
    } else {
        CursorIcon::ResizeVertical
    };
    let held = response.dragged();
    let active = response.hovered() || held;
    if active {
        ui.ctx().set_cursor_icon(cursor);
    }
    let shown = controls::animate(ui.ctx(), id.with("grip"), active, theme::motion::HOVER);
    if shown > 0.0 {
        let grip = if vertical {
            Rect::from_center_size(rect.center(), vec2(3.0, 40.0))
        } else {
            Rect::from_center_size(rect.center(), vec2(40.0, 3.0))
        };
        let ink = if held { p.accent } else { p.muted };
        ui.painter()
            .rect_filled(grip, CornerRadius::same(2), ink.gamma_multiply(shown));
    }
    let delta = if held {
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
