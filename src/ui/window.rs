//! The window's own frame. Neptune Video draws its titlebar: the toolbar moves the
//! window, three lights at its leading edge close, minimize and maximize it,
//! and the edges resize it. Nothing here touches the project; closing goes
//! through the same request as the system's, so unsaved work is still asked
//! about.
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, ResizeDirection, Sense, Stroke, Ui, Vec2, WidgetInfo,
    WidgetType, vec2,
};

/// How far the toolbar's own controls start from the window's leading edge:
/// clear of the lights.
pub const LEADING: f32 = 70.0;

/// The window's corner for its state. Windows rounds and outlines the window
/// itself; elsewhere a maximized or fullscreen window is square, except that
/// macOS keeps a zoomed window's corners.
fn radius_for(fullscreen: bool, maximized: bool) -> u8 {
    let rounded =
        !cfg!(target_os = "windows") && !fullscreen && (cfg!(target_os = "macos") || !maximized);
    if rounded { metrics::WINDOW_RADIUS } else { 0 }
}

pub fn corner_radius(ctx: &egui::Context) -> u8 {
    ctx.input(|input| {
        let viewport = input.viewport();
        radius_for(
            viewport.fullscreen.unwrap_or(false),
            viewport.maximized.unwrap_or(false),
        )
    })
}

fn toggle_maximized(ctx: &egui::Context) {
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}

/// Moves the window from `rect`, and maximizes or restores it on a double
/// click. Registered before the controls drawn over it, which take their
/// own presses.
pub fn drag_region(ui: &mut Ui, rect: Rect) {
    let drag = ui.interact(rect, ui.id().with("window-drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        toggle_maximized(ui.ctx());
    }
}

/// Close, minimize and maximize as three quiet lights. Their glyphs appear
/// when the pointer approaches, and they fade to neutral in an inactive
/// window.
pub fn controls(ui: &mut Ui, left_center: Pos2, p: &Palette) {
    let group = Rect::from_min_max(
        left_center + vec2(-6.0, -12.0),
        left_center + vec2(58.0, 12.0),
    );
    let hovered = ui.rect_contains_pointer(group);
    let active = ui.input(|input| input.focused);
    let reveal = ui
        .ctx()
        .animate_bool_with_time(ui.id().with("window-controls"), hovered, 0.12);
    for (index, (label, rgb)) in [
        ("Close window", 0xff5f57),
        ("Minimize", 0xfebc2e),
        ("Maximize", 0x28c840),
    ]
    .into_iter()
    .enumerate()
    {
        let center = left_center + vec2(6.0 + index as f32 * 20.0, 0.0);
        let response = ui.interact(
            Rect::from_center_size(center, Vec2::splat(20.0)),
            ui.id().with(("window-control", label)),
            Sense::click(),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
        let color = theme::color(rgb);
        let painter = ui.painter();
        let fill = if active || hovered {
            color
        } else {
            theme::mix(p.chrome, p.muted, 0.55)
        };
        painter.circle_filled(
            center,
            6.0,
            if response.is_pointer_button_down_on() {
                theme::mix(fill, Color32::BLACK, 0.2)
            } else {
                fill
            },
        );
        painter.circle_stroke(center, 6.0, Stroke::new(0.5, Color32::from_black_alpha(48)));
        if response.has_focus() {
            painter.circle_stroke(center, 8.5, Stroke::new(1.5, theme::tint(p.accent, 0.8)));
        }
        if reveal > 0.0 {
            icons::paint(
                painter,
                Rect::from_center_size(center, Vec2::splat(10.0)),
                match index {
                    0 => Icon::Close,
                    1 => Icon::Minus,
                    _ => Icon::Plus,
                },
                theme::tint(theme::mix(color, Color32::BLACK, 0.62), reveal),
            );
        }
        if response.clicked() {
            let ctx = ui.ctx();
            match index {
                0 => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                1 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                _ => toggle_maximized(ctx),
            }
        }
        response.on_hover_text(label);
    }
}

/// The window's outline and its resize edges, over everything else in the
/// window's layer.
pub fn frame(ui: &mut Ui, bounds: Rect, p: &Palette) {
    let radius = corner_radius(ui.ctx());
    if radius > 0 {
        ui.painter().rect_stroke(
            bounds,
            radius,
            Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
    }
    if ui.input(|input| input.viewport().maximized.unwrap_or(false)) {
        return;
    }
    for (direction, rect, cursor) in resize_zones(bounds) {
        let response = ui
            .interact(
                rect,
                ui.id().with(("window-resize", direction as u8)),
                Sense::drag(),
            )
            .on_hover_cursor(cursor);
        if response.drag_started() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

/// A 4-point band along each edge, and an 8-point square at each corner.
fn resize_zones(bounds: Rect) -> [(ResizeDirection, Rect, CursorIcon); 8] {
    use ResizeDirection::*;
    let edge = 4.0;
    let corner = 8.0;
    let wide = bounds.width() - 2.0 * corner;
    let tall = bounds.height() - 2.0 * corner;
    [
        (
            North,
            Rect::from_min_size(bounds.min + vec2(corner, 0.0), vec2(wide, edge)),
            CursorIcon::ResizeVertical,
        ),
        (
            South,
            Rect::from_min_size(bounds.left_bottom() + vec2(corner, -edge), vec2(wide, edge)),
            CursorIcon::ResizeVertical,
        ),
        (
            West,
            Rect::from_min_size(bounds.min + vec2(0.0, corner), vec2(edge, tall)),
            CursorIcon::ResizeHorizontal,
        ),
        (
            East,
            Rect::from_min_size(bounds.right_top() + vec2(-edge, corner), vec2(edge, tall)),
            CursorIcon::ResizeHorizontal,
        ),
        (
            NorthWest,
            Rect::from_min_size(bounds.min, Vec2::splat(corner)),
            CursorIcon::ResizeNwSe,
        ),
        (
            NorthEast,
            Rect::from_min_size(bounds.right_top() - vec2(corner, 0.0), Vec2::splat(corner)),
            CursorIcon::ResizeNeSw,
        ),
        (
            SouthWest,
            Rect::from_min_size(
                bounds.left_bottom() - vec2(0.0, corner),
                Vec2::splat(corner),
            ),
            CursorIcon::ResizeNeSw,
        ),
        (
            SouthEast,
            Rect::from_min_size(bounds.max - Vec2::splat(corner), Vec2::splat(corner)),
            CursorIcon::ResizeNwSe,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_rounded_only_where_the_system_does_not_round_it() {
        let rounded = if cfg!(target_os = "windows") {
            0
        } else {
            metrics::WINDOW_RADIUS
        };
        assert_eq!(radius_for(false, false), rounded);
        assert_eq!(radius_for(true, false), 0);
        assert_eq!(radius_for(true, true), 0);
        let zoomed = if cfg!(target_os = "macos") {
            rounded
        } else {
            0
        };
        assert_eq!(radius_for(false, true), zoomed);
    }

    #[test]
    fn resize_zones_hug_the_edges_without_overlapping() {
        let bounds = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let zones = resize_zones(bounds);
        for (index, (direction, rect, _)) in zones.iter().enumerate() {
            assert!(
                bounds.contains_rect(*rect),
                "{direction:?} leaves the window"
            );
            for (_, other, _) in &zones[index + 1..] {
                assert!(
                    rect.intersect(*other).area() <= 0.0,
                    "{direction:?} overlaps another zone"
                );
            }
        }
        // The toolbar's controls start below the top band.
        let north = zones[0].1;
        assert!(north.bottom() <= 4.0);
    }
}
