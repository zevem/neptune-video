//! The curve editor: a tone curve drawn as the compositor evaluates it, with
//! its points to take hold of.
//!
//! The line is `compose::pass::curve_at` over `compose::pass::slopes`, which
//! is what the shader is given, so what is drawn here is what the picture
//! gets. A press on the plot adds a point and holds it; a point is dragged,
//! moved with the arrows while it has the keyboard, and removed with a double
//! click or Delete. The ends stay at the inputs they have, and a point never
//! passes its neighbours.
use crate::{
    compose::pass,
    theme::{self, Palette},
};
use editor_core::look;
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, Key, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui,
    Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

/// The curve that changes nothing.
pub(super) const IDENTITY: [[f32; 2]; 2] = [[0.0, 0.0], [1.0, 1.0]];
/// The least two neighbouring points are apart along the input.
const GAP: f32 = 0.01;
/// The side of a point's target, and the radius it is drawn at.
const TARGET: f32 = 20.0;
const RADIUS: f32 = 4.5;
/// The line is drawn through this many places.
const STEPS: usize = 64;

/// What a frame of the editor came to.
pub(super) struct Edited {
    /// The points as changed, when they were.
    pub points: Option<Vec<[f32; 2]>>,
    /// One of the points has the keyboard: the arrows and Delete are its.
    pub focused: bool,
}

/// `points` with the one at `index` moved to `to`, as far as its neighbours
/// and the unit square let it go. An end keeps its input.
pub(super) fn moved(points: &[[f32; 2]], index: usize, to: [f32; 2]) -> Vec<[f32; 2]> {
    let mut out = points.to_vec();
    let Some(last) = points.len().checked_sub(1) else {
        return out;
    };
    if index > last {
        return out;
    }
    let x = if index == 0 || index == last {
        points[index][0]
    } else {
        let (low, high) = (points[index - 1][0] + GAP, points[index + 1][0] - GAP);
        // Neighbours closer than two gaps leave it no room along the input.
        if low <= high {
            to[0].clamp(low, high)
        } else {
            points[index][0]
        }
    };
    out[index] = [x, to[1].clamp(0.0, 1.0)];
    out
}

/// `points` with one more at `at`, and its index, when the curve has room
/// for it there: fewer points than it may hold, and none within a gap.
pub(super) fn added(points: &[[f32; 2]], at: [f32; 2]) -> Option<(Vec<[f32; 2]>, usize)> {
    if points.len() >= look::MAX_CURVE_POINTS {
        return None;
    }
    let x = at[0].clamp(0.0, 1.0);
    if points.iter().any(|point| (point[0] - x).abs() < GAP) {
        return None;
    }
    let index = points.partition_point(|point| point[0] < x);
    let mut out = points.to_vec();
    out.insert(index, [x, at[1].clamp(0.0, 1.0)]);
    Some((out, index))
}

/// A point on the curve in the middle of its widest stretch: where one is
/// added without a pointer.
pub(super) fn added_midway(points: &[[f32; 2]]) -> Option<(Vec<[f32; 2]>, usize)> {
    let widest =
        (points.windows(2)).max_by(|a, b| (a[1][0] - a[0][0]).total_cmp(&(b[1][0] - b[0][0])))?;
    let x = (widest[0][0] + widest[1][0]) * 0.5;
    added(
        points,
        [x, pass::curve_at(points, &pass::slopes(points), x)],
    )
}

/// `points` without the one at `index`. An end is not removed, and nor is
/// one of the last two.
pub(super) fn removed(points: &[[f32; 2]], index: usize) -> Option<Vec<[f32; 2]>> {
    (index > 0 && index + 1 < points.len() && points.len() > 2).then(|| {
        let mut out = points.to_vec();
        out.remove(index);
        out
    })
}

/// Draws the curve through `points` in `ink` and takes what is done to it.
/// `selected` is the point last taken hold of; `name` says which curve it
/// is, for the names its points are given.
pub(super) fn editor(
    ui: &mut Ui,
    p: &Palette,
    points: &[[f32; 2]],
    ink: Color32,
    selected: &mut usize,
    name: &str,
) -> Edited {
    let width = ui.available_width();
    let size = vec2(width, width.clamp(120.0, 190.0));
    let (rect, plot_response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    plot_response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Other, ui.is_enabled(), format!("{name} curve"))
    });
    // The points on the edges are whole inside the frame.
    let plot = rect.shrink(RADIUS + 3.0);
    let to_screen = |[x, y]: [f32; 2]| {
        pos2(
            plot.left() + x * plot.width(),
            plot.bottom() - y * plot.height(),
        )
    };
    let from_screen = |pos: Pos2| {
        [
            ((pos.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0),
            ((plot.bottom() - pos.y) / plot.height().max(1.0)).clamp(0.0, 1.0),
        ]
    };
    let count = points.len();
    *selected = (*selected).min(count.saturating_sub(1));
    let point_id = |index: usize| ui.id().with(("curve-point", name, index));
    let mut next: Option<Vec<[f32; 2]>> = None;
    let mut focused = false;

    // A press on the plot is a new point, held from then on.
    if plot_response.drag_started() || plot_response.clicked() {
        if let Some(pos) = plot_response.interact_pointer_pos()
            && let Some((with, index)) = added(points, from_screen(pos))
        {
            *selected = index;
            ui.memory_mut(|memory| memory.request_focus(point_id(index)));
            next = Some(with);
        }
    } else if plot_response.dragged()
        && let Some(pos) = plot_response.interact_pointer_pos()
    {
        next = Some(moved(points, *selected, from_screen(pos)));
    }

    let mut responses = Vec::with_capacity(count);
    for (index, point) in points.iter().enumerate() {
        let target = Rect::from_center_size(to_screen(*point), Vec2::splat(TARGET));
        let target = super::clear_of_bar(ui, target);
        let response = ui.interact(target, point_id(index), Sense::click_and_drag());
        let enabled = ui.is_enabled();
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(
                WidgetType::Other,
                enabled,
                format!("{name} curve point {} of {count}", index + 1),
            );
            info.current_text_value = Some(format!(
                "{:.0} % in, {:.0} % out",
                point[0] * 100.0,
                point[1] * 100.0
            ));
            info
        });
        if response.drag_started() || response.clicked() {
            *selected = index;
            response.request_focus();
        }
        if response.double_clicked() {
            next = removed(points, index).or(next);
        } else if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            next = Some(moved(points, index, from_screen(pos)));
        }
        if response.has_focus() {
            focused = true;
            *selected = index;
            // The arrows move the point and do not leave it.
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
            // A key is read with the modifiers of its own event: a whole
            // Shift+→ can arrive in one frame, and by its end Shift is up.
            let (step, remove) = ui.input(|i| {
                let mut step = Vec2::ZERO;
                let mut remove = false;
                for event in &i.events {
                    let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    else {
                        continue;
                    };
                    let size = if modifiers.shift { 0.1 } else { 0.01 };
                    match key {
                        Key::ArrowRight => step.x += size,
                        Key::ArrowLeft => step.x -= size,
                        Key::ArrowUp => step.y += size,
                        Key::ArrowDown => step.y -= size,
                        Key::Delete | Key::Backspace => remove = true,
                        _ => {}
                    }
                }
                (step, remove)
            });
            if remove {
                next = removed(points, index).or(next);
            } else if step != Vec2::ZERO {
                next = Some(moved(points, index, [point[0] + step.x, point[1] + step.y]));
            }
        }
        responses.push(response);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        // A well in the card: the timeline's ground, edged by a separator.
        let radius = CornerRadius::same(6);
        painter.rect_filled(rect, radius, p.bg);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, p.separator),
            StrokeKind::Inside,
        );
        // The separator at six tenths of its own strength: it is a
        // translucent ink, which `tint` would make solid.
        let grid = Stroke::new(1.0, p.separator.gamma_multiply(0.6));
        for quarter in 1..4 {
            let at = quarter as f32 * 0.25;
            painter.line_segment([to_screen([at, 0.0]), to_screen([at, 1.0])], grid);
            painter.line_segment([to_screen([0.0, at]), to_screen([1.0, at])], grid);
        }
        // The curve that changes nothing, to read the other against.
        painter.line_segment(
            [to_screen(IDENTITY[0]), to_screen(IDENTITY[1])],
            Stroke::new(1.0, theme::tint(p.muted, 0.5)),
        );
        let enabled = ui.is_enabled();
        let ink = if enabled { ink } else { p.muted };
        let shown = next.as_deref().unwrap_or(points);
        let slopes = pass::slopes(shown);
        let line: Vec<Pos2> = (0..=STEPS)
            .map(|step| {
                let x = step as f32 / STEPS as f32;
                to_screen([x, pass::curve_at(shown, &slopes, x).clamp(0.0, 1.0)])
            })
            .collect();
        painter.add(Shape::line(line, Stroke::new(1.5, ink)));
        // The point last taken hold of is marked only while the person is
        // at the curve: holding it, or with a point at the keyboard. At
        // rest, in a clip no one has touched, no point is chosen.
        let picked = plot_response.dragged()
            || plot_response.is_pointer_button_down_on()
            || (responses.iter()).any(|r| r.has_focus() || r.dragged());
        // Drawn where they are held now; the targets follow a frame later.
        let mut capsule = None;
        for (index, point) in shown.iter().enumerate() {
            let centre = to_screen(*point);
            let response = responses.get(index).filter(|_| shown.len() == count);
            let held = response.is_some_and(|r| r.dragged() || r.is_pointer_button_down_on());
            let hovered = response.is_some_and(egui::Response::hovered);
            let size = if held || hovered { 5.0 } else { 4.0 };
            let chosen = picked && index == *selected;
            let fill = if chosen && enabled { p.accent } else { p.bg };
            painter.circle_filled(centre, size, fill);
            let rim = if enabled { p.fg } else { p.muted };
            painter.circle_stroke(centre, size, Stroke::new(1.5, rim));
            if response.is_some_and(egui::Response::has_focus) {
                painter.circle_stroke(
                    centre,
                    RADIUS + 3.5,
                    Stroke::new(2.0, theme::tint(p.accent, 0.55)),
                );
            }
            if held {
                let text = format!("{:.0} → {:.0}", point[0] * 100.0, point[1] * 100.0);
                capsule = Some((centre, text));
            }
        }
        if let Some((centre, text)) = capsule {
            super::speed::value_capsule(&painter, p, rect, centre, &text);
        }
    }
    for response in responses {
        let cursor = if response.dragged() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        };
        response.on_hover_cursor(cursor);
    }
    plot_response.on_hover_cursor(CursorIcon::Crosshair);
    Edited {
        points: next,
        focused,
    }
}
