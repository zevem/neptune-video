//! The window's own frame. Neptune Video draws its titlebar: the toolbar moves the
//! window, three lights at its leading edge close, minimize and maximize it,
//! and the edges resize it. Nothing here touches the project; closing goes
//! through the same request as the system's, so unsaved work is still asked
//! about.
use super::{
    controls::{self, Level},
    kit,
};
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Pos2, Rect, ResizeDirection, Sense, Stroke, Ui, Vec2,
    WidgetInfo, WidgetType, vec2,
};

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

/// The interface's size as it was last changed, said under the toolbar.
#[derive(Clone)]
struct InterfaceSize {
    level: Level,
    /// The size asked for was more than the window has room for.
    capped: bool,
}

fn interface_size_id() -> Id {
    Id::new("interface-size")
}

/// Says the interface's new size for a moment under the toolbar: `scale`,
/// and whether it was held below the size asked for.
pub fn say_interface_size(ctx: &egui::Context, scale: f32, capped: bool) {
    let said = InterfaceSize {
        level: Level {
            name: "Interface size",
            value: format!("{:.0}%", scale * 100.0),
            shown: ctx.input(|input| input.time),
        },
        capped,
    };
    ctx.data_mut(|data| data.insert_temp(interface_size_id(), said));
}

/// Terminal's level capsule for the interface's size, while it is said: the
/// name, the value and, when the size was held down, why. It holds still and
/// lets the window sleep, then fades; it takes no input.
pub fn interface_size(ctx: &egui::Context, p: &Palette) {
    let Some(said) = ctx.data(|data| data.get_temp::<InterfaceSize>(interface_size_id())) else {
        return;
    };
    let level = &said.level;
    let now = ctx.input(|input| input.time);
    let hold = level.shown + HOLD - now;
    // Under Reduce motion it holds and is gone, with no frames to fade in.
    let still = kit::reduced_motion(ctx);
    let opacity = if still {
        (hold > 0.0).then_some(1.0)
    } else {
        level.opacity(now)
    };
    let Some(opacity) = opacity else {
        ctx.data_mut(|data| data.remove::<InterfaceSize>(interface_size_id()));
        return;
    };
    // Sleep through the hold; repaint only for the fade.
    if hold > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(hold));
    } else {
        ctx.request_repaint();
    }
    const NOTE: &str = ", the largest this window has room for";
    egui::Area::new(controls::viewport_id(ctx, Id::new("neptune-level")))
        .order(egui::Order::Tooltip)
        .interactable(false)
        // It fades by its own clock, so that the hold asks for no frames.
        .fade_in(false)
        .anchor(Align2::CENTER_TOP, vec2(0.0, metrics::TOOLBAR_HEIGHT + 8.0))
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            let painter = ui.painter().clone();
            let name = painter.layout_no_wrap(level.name.into(), theme::medium(12.0), p.secondary);
            let value = painter.layout_no_wrap(level.value.clone(), theme::medium(12.0), p.fg);
            let note = said
                .capped
                .then(|| painter.layout_no_wrap(NOTE.into(), theme::regular(12.0), p.secondary));
            let note_width = note.as_ref().map_or(0.0, |note| note.size().x);
            let (rect, response) = ui.allocate_exact_size(
                vec2(name.size().x + value.size().x + note_width + 36.0, 28.0),
                Sense::hover(),
            );
            response.widget_info(|| {
                let mut label = format!("{} {}", level.name, level.value);
                if said.capped {
                    label.push_str(NOTE);
                }
                WidgetInfo::labeled(WidgetType::Label, true, label)
            });
            controls::capsule(&painter, rect, *p);
            let name = controls::galley_at(
                &painter,
                Pos2::new(rect.left() + 14.0, rect.center().y),
                name,
            );
            let value = controls::galley_at(
                &painter,
                Pos2::new(name.right() + 8.0, rect.center().y),
                value,
            );
            if let Some(note) = note {
                controls::galley_at(&painter, Pos2::new(value.right(), rect.center().y), note);
            }
        });
}

/// How long the capsule holds before it fades: Terminal's level's.
const HOLD: f64 = 1.2;

/// A side panel's slide in or out, Terminal's `SidebarSlide`: it starts from
/// however much of the panel was showing, so a toggle reversed midway turns
/// around without a jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    /// The state slid toward.
    shown: bool,
    /// How much showed as it set off, and when.
    from: f32,
    started: f64,
}

impl Slide {
    /// A panel at rest, shown or not.
    pub fn settled(shown: bool) -> Self {
        Self {
            shown,
            from: f32::from(u8::from(shown)),
            started: f64::NEG_INFINITY,
        }
    }

    /// The slide toward `shown` from `now` on. A change sets off from where
    /// the panel is; the same state goes on as it was.
    pub fn toward(self, shown: bool, now: f64) -> Self {
        if shown == self.shown {
            return self;
        }
        Self {
            shown,
            from: self.reveal(now),
            started: now,
        }
    }

    /// How far through the slide it is at `now`, from 0; 1 and on once it
    /// has settled.
    fn progress(self, now: f64) -> f32 {
        ((now - self.started) / f64::from(theme::motion::PANEL)) as f32
    }

    /// How much of the panel shows at `now`: 0 hidden, 1 shown.
    pub fn reveal(self, now: f64) -> f32 {
        let to = f32::from(u8::from(self.shown));
        let progress = self.progress(now);
        if progress >= 1.0 {
            to
        } else {
            egui::lerp(
                self.from..=to,
                egui::emath::easing::cubic_out(progress.max(0.0)),
            )
        }
    }

    /// Whether it still moves at `now`: until the frame it settles in.
    pub fn moving(self, now: f64) -> bool {
        self.progress(now) < 1.0
    }
}

/// How much of the side panel `id` shows in this frame, slid toward `shown`
/// over 0.16 s with `cubic_out`. Frames are asked for only while it moves,
/// and under Reduce motion it is where it is going at once.
pub fn slide(ctx: &egui::Context, id: Id, shown: bool) -> f32 {
    let id = controls::viewport_id(ctx, id);
    let now = ctx.input(|input| input.time);
    let slide = if kit::reduced_motion(ctx) {
        Slide::settled(shown)
    } else {
        ctx.data(|data| data.get_temp::<Slide>(id))
            .unwrap_or(Slide::settled(shown))
            .toward(shown, now)
    };
    if slide.moving(now) {
        ctx.request_repaint();
    }
    ctx.data_mut(|data| data.insert_temp(id, slide));
    slide.reveal(now)
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
    fn a_slide_reversed_midway_turns_around_without_a_jump() {
        // Spec §2.5, as Terminal's sidebar: hidden from shown at 10 s.
        let hide = Slide::settled(true).toward(false, 10.0);
        assert_eq!(hide.reveal(10.0), 1.0);
        let midway = hide.reveal(10.08);
        assert!(0.0 < midway && midway < 1.0, "{midway}");
        assert_eq!(hide.reveal(10.2), 0.0);
        // Shown again halfway: it sets off from where it was.
        let show = hide.toward(true, 10.08);
        assert_eq!(show.reveal(10.08), midway);
        assert!(show.reveal(10.12) > midway);
        assert_eq!(show.reveal(10.3), 1.0);
        assert!(show.moving(10.2) && !show.moving(10.3));
        // Asked for the state it is in, it goes on as it was.
        assert_eq!(show.toward(true, 10.1), show);
        assert!(!Slide::settled(false).moving(0.0));
    }

    /// How much of the panel `slide` shows in the frame at `time`, asked
    /// toward `shown`, and how long the window may sleep after.
    fn slid_at(
        ctx: &egui::Context,
        time: f64,
        shown: bool,
        reduced: bool,
    ) -> (f32, std::time::Duration) {
        let mut reveal = f32::NAN;
        let output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                kit::set_reduced_motion(ui.ctx(), reduced);
                reveal = slide(ui.ctx(), Id::new("panel"), shown);
            },
        );
        let delay = (output.viewport_output.values())
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::MAX);
        output.drop_without_applying_deltas();
        (reveal, delay)
    }

    #[test]
    fn a_panel_settles_in_its_slide_and_at_once_under_reduce_motion() {
        // §2.5 and §9.12: a panel shown at rest asks for nothing.
        let ctx = egui::Context::default();
        // A fresh context asks for its own first frames.
        slid_at(&ctx, 0.9, true, false);
        slid_at(&ctx, 0.95, true, false);
        assert_eq!(
            slid_at(&ctx, 1.0, true, false),
            (1.0, std::time::Duration::MAX)
        );
        // Hidden at 2 s, it slides away in frames asked for as it goes, and
        // has settled by 0.16 s later. egui draws one frame more after the
        // last that was asked for; after that the window sleeps.
        let (reveal, delay) = slid_at(&ctx, 2.0, false, false);
        assert_eq!((reveal, delay), (1.0, std::time::Duration::ZERO));
        let (reveal, delay) = slid_at(&ctx, 2.08, false, false);
        assert!(0.0 < reveal && reveal < 1.0, "{reveal}");
        assert_eq!(delay, std::time::Duration::ZERO);
        let settled = 2.0 + f64::from(theme::motion::PANEL);
        assert_eq!(slid_at(&ctx, settled, false, false).0, 0.0);
        assert_eq!(
            slid_at(&ctx, settled + 0.02, false, false),
            (0.0, std::time::Duration::MAX)
        );
        // Under Reduce motion it is where it is going in the first frame,
        // and asks for no other.
        assert_eq!(
            slid_at(&ctx, 4.0, true, true),
            (1.0, std::time::Duration::MAX)
        );
        assert_eq!(
            slid_at(&ctx, 4.05, false, true),
            (0.0, std::time::Duration::MAX)
        );
    }

    #[test]
    fn a_window_is_rounded_only_where_the_system_does_not_round_it() {
        // Spec §3.4: Terminal's 16-point window corner.
        assert_eq!(metrics::WINDOW_RADIUS, 16);
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

    /// The accessible names the frame at `time` gives, and how long it may
    /// sleep after.
    fn said_at(ctx: &egui::Context, p: &Palette, time: f64) -> (Vec<String>, std::time::Duration) {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                time: Some(time),
                ..Default::default()
            },
            |ui| interface_size(ui.ctx(), p),
        );
        let names = (output.platform_output.accesskit_update.iter())
            .flat_map(|update| update.nodes.iter())
            // A label says its words as its value.
            .filter_map(|(_, node)| node.label().or_else(|| node.value()).map(str::to_owned))
            .collect();
        let delay = (output.viewport_output.values())
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::MAX);
        output.drop_without_applying_deltas();
        (names, delay)
    }

    #[test]
    fn the_interface_size_is_said_in_a_capsule_that_lets_the_window_sleep() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        ctx.enable_accesskit();
        let p = Palette::new(theme::Appearance::Dark, theme::Accent::default());
        let (names, _) = said_at(&ctx, &p, 1.0);
        assert!(names.is_empty(), "nothing is said before a change");
        ctx.run_ui(
            egui::RawInput {
                time: Some(1.0),
                ..Default::default()
            },
            |ui| say_interface_size(ui.ctx(), 1.2, false),
        )
        .drop_without_applying_deltas();
        // A new area is measured, unseen, in its first frames.
        said_at(&ctx, &p, 1.05);
        said_at(&ctx, &p, 1.1);
        let (names, delay) = said_at(&ctx, &p, 1.15);
        assert!(
            names.iter().any(|name| name == "Interface size 120%"),
            "{names:?}"
        );
        // It holds still: the window sleeps through the hold.
        assert!(delay > std::time::Duration::from_millis(500), "{delay:?}");
        // A size held down by the window says why, in today's words.
        ctx.run_ui(
            egui::RawInput {
                time: Some(2.0),
                ..Default::default()
            },
            |ui| say_interface_size(ui.ctx(), 1.5, true),
        )
        .drop_without_applying_deltas();
        said_at(&ctx, &p, 2.05);
        let (names, _) = said_at(&ctx, &p, 2.1);
        assert!(
            names
                .iter()
                .any(|name| name == "Interface size 150%, the largest this window has room for"),
            "{names:?}"
        );
        // And then it has gone, and asks for nothing.
        let (names, delay) = said_at(&ctx, &p, 4.0);
        assert!(!names.iter().any(|name| name.starts_with("Interface size")));
        assert_eq!(delay, std::time::Duration::MAX);
    }
}
