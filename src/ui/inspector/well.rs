//! The colour well: a swatch of a colour that opens a picker of its own.
//!
//! The swatch shows the colour over a checkerboard, so that a translucent
//! colour reads as one. Its popover holds a field of saturation and
//! brightness, a strip of hues, a strip of opacity where the colour has one,
//! the colour's hex, and the person's brand colours where they are offered.
//! A drag in the popover changes the colour on every frame of it, as egui's
//! picker did: the inspector's draft makes the whole drag one step.
use super::controls;
use crate::{
    theme::{self, Palette, metrics},
    ui::text_tab,
};
use eframe::egui::{
    self, Color32, CursorIcon, Key, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

/// The swatch, and the squares of the checkerboard under a colour.
const SWATCH: Vec2 = vec2(36.0, 22.0);
const CHECK: f32 = 6.0;
/// The popover's content: as wide as its field, inside 12 of padding.
const FIELD: Vec2 = vec2(208.0, 140.0);
const STRIP: f32 = 12.0;
const SWATCH_RADIUS: u8 = 6;
/// A brand colour's swatch in the popover.
const BRAND: f32 = 20.0;

/// A colour as the picker holds it: the hue, saturation and value of its
/// sRGB channels and its opacity, each from nought to one. A grey keeps the
/// hue it was dragged from, which its channels no longer say.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsv {
    h: f32,
    s: f32,
    v: f32,
    a: f32,
}

impl Hsv {
    fn of([r, g, b, a]: [u8; 4]) -> Self {
        let [r, g, b] = [r, g, b].map(|channel| f32::from(channel) / 255.0);
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        let h = if delta <= 0.0 {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };
        let s = if max <= 0.0 { 0.0 } else { delta / max };
        Self {
            h,
            s,
            v: max,
            a: f32::from(a) / 255.0,
        }
    }

    fn srgba(self) -> [u8; 4] {
        let sixth = self.h.rem_euclid(1.0) * 6.0;
        let chroma = self.v * self.s;
        let x = chroma * (1.0 - (sixth % 2.0 - 1.0).abs());
        let (r, g, b) = match sixth as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let low = self.v - chroma;
        let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
        [byte(r + low), byte(g + low), byte(b + low), byte(self.a)]
    }

    /// The colour at full opacity, as the field and the strips draw it.
    fn solid(self) -> Color32 {
        let [r, g, b, _] = self.srgba();
        Color32::from_rgb(r, g, b)
    }
}

fn shown([r, g, b, a]: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The colour's hex, with its opacity where the colour has one.
fn hex(color: [u8; 4], alpha: bool) -> String {
    let [r, g, b, a] = color;
    if alpha && a < 255 {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// A typed hex: six digits, or eight with the opacity, after an optional #.
fn parse(text: &str, opacity: u8) -> Option<[u8; 4]> {
    let digits = text.trim().trim_start_matches('#');
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    match digits.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, opacity]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// A checkerboard of `CHECK` squares over `rect`, its corners rounded by
/// `radius`: the squares the rounding cuts are left in the lighter ground.
fn checker(painter: &Painter, p: &Palette, rect: Rect, radius: u8) {
    let (light, dark) = p.checker();
    painter.rect_filled(rect, radius, light);
    let columns = (rect.width() / CHECK).ceil() as i32;
    let rows = (rect.height() / CHECK).ceil() as i32;
    for column in 0..columns {
        for row in 0..rows {
            let corner = (column == 0 || column == columns - 1) && (row == 0 || row == rows - 1);
            if (column + row) % 2 == 0 || (corner && radius > 0) {
                continue;
            }
            let square = Rect::from_min_size(
                rect.min + vec2(column as f32, row as f32) * CHECK,
                Vec2::splat(CHECK),
            )
            .intersect(rect);
            painter.rect_filled(square, 0, dark);
        }
    }
}

/// Rounds the corners of what was drawn square into `rect`, with quarter
/// arcs of the ground it sits on: `kit::corner_caps` at a control's radius.
fn round_corners(painter: &Painter, rect: Rect, radius: f32, ground: Color32) {
    let inset = rect.shrink(radius);
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
                centre + vec2(angle.cos(), angle.sin()) * (radius + 2.0)
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(4.0, ground)));
    }
}

/// A mesh over `rect` of `columns` by `rows` cells, each corner coloured by
/// `color` at its place across and down, from nought to one.
fn gradient(rect: Rect, columns: usize, rows: usize, color: impl Fn(f32, f32) -> Color32) -> Mesh {
    let mut mesh = Mesh::default();
    for row in 0..=rows {
        for column in 0..=columns {
            let (x, y) = (column as f32 / columns as f32, row as f32 / rows as f32);
            mesh.colored_vertex(rect.lerp_inside(vec2(x, y)), color(x, y));
        }
    }
    let across = columns as u32 + 1;
    for row in 0..rows as u32 {
        for column in 0..columns as u32 {
            let corner = row * across + column;
            mesh.add_triangle(corner, corner + 1, corner + across);
            mesh.add_triangle(corner + 1, corner + across + 1, corner + across);
        }
    }
    mesh
}

/// A ring the pointer holds a place in a field or a strip by.
fn handle(painter: &Painter, centre: Pos2, radius: f32, fill: Color32) {
    painter.circle_filled(centre, radius, fill);
    painter.circle_stroke(centre, radius, Stroke::new(2.0, Color32::WHITE));
    painter.circle_stroke(
        centre,
        radius + 1.0,
        Stroke::new(1.0, Color32::from_black_alpha(90)),
    );
}

/// Where the frame in which a part of a picker last had the keyboard is
/// kept, for [`had_keys`].
fn keys_id() -> egui::Id {
    egui::Id::new("colour-well-keys")
}

/// The frame in which a part of a colour well's picker last had the
/// keyboard: the arrows are then its own, and not the application's.
pub(super) fn had_keys(ctx: &egui::Context) -> Option<u64> {
    ctx.data(|data| data.get_temp(keys_id()))
}

/// The arrows on a part of the picker that has the keyboard: a hundredth a
/// press, a tenth with Shift. Across and up.
fn arrows(ui: &Ui, response: &Response) -> Vec2 {
    if !response.has_focus() {
        return Vec2::ZERO;
    }
    let frame = ui.ctx().cumulative_frame_nr();
    ui.data_mut(|data| data.insert_temp(keys_id(), frame));
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
    ui.input(|i| {
        let mut step = Vec2::ZERO;
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
                _ => {}
            }
        }
        step
    })
}

/// Where the pointer holds a part of the picker drawn in `rect`, across and
/// down from nought to one.
fn held(response: &Response, rect: Rect) -> Option<Vec2> {
    let pos = response.interact_pointer_pos()?;
    (response.dragged() || response.is_pointer_button_down_on()).then(|| {
        vec2(
            ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0),
            ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0),
        )
    })
}

/// The edge of a part of the picker drawn in `rect`, and its ring while it
/// has the keyboard.
fn frame(painter: &Painter, p: &Palette, response: &Response, rect: Rect, radius: u8) {
    painter.rect_stroke(rect, radius, p.hairline(), StrokeKind::Inside);
    if response.has_focus() {
        controls::focus_ring(painter, rect, radius, *p);
    }
}

/// The saturation across and the brightness up of the colour's hue.
fn field(ui: &mut Ui, p: &Palette, hsv: &mut Hsv) -> bool {
    let (rect, response) = ui.allocate_exact_size(FIELD, Sense::click_and_drag());
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        let mut info =
            WidgetInfo::labeled(WidgetType::Slider, enabled, "Saturation and brightness");
        info.value = Some(f64::from(hsv.s * 100.0));
        info
    });
    let before = *hsv;
    if let Some(at) = held(&response, rect) {
        hsv.s = at.x;
        hsv.v = 1.0 - at.y;
    }
    let step = arrows(ui, &response);
    hsv.s = (hsv.s + step.x).clamp(0.0, 1.0);
    hsv.v = (hsv.v + step.y).clamp(0.0, 1.0);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hue = *hsv;
        painter.add(gradient(rect, 16, 12, |x, y| {
            Hsv {
                s: x,
                v: 1.0 - y,
                ..hue
            }
            .solid()
        }));
        round_corners(painter, rect, f32::from(SWATCH_RADIUS), p.elevated);
        frame(painter, p, &response, rect, SWATCH_RADIUS);
        let at = pos2(
            rect.left() + hsv.s * rect.width(),
            rect.top() + (1.0 - hsv.v) * rect.height(),
        );
        handle(painter, at, 6.0, hsv.solid());
    }
    response.on_hover_cursor(CursorIcon::Crosshair);
    *hsv != before
}

/// A strip's place: drawn `STRIP` tall, in the middle of a target as tall
/// as a small button, which the pointer takes. The rect is the drawn one.
fn strip(ui: &mut Ui) -> (Rect, Response) {
    let (target, response) = ui.allocate_exact_size(
        vec2(FIELD.x, metrics::SMALL_BUTTON),
        Sense::click_and_drag(),
    );
    let rect = Rect::from_center_size(target.center(), vec2(FIELD.x, STRIP));
    (rect, response)
}

/// The hues, red round to red, across a strip.
fn hues(ui: &mut Ui, p: &Palette, hsv: &mut Hsv) -> bool {
    let (rect, response) = strip(ui);
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Slider, enabled, "Hue");
        info.value = Some(f64::from(hsv.h * 360.0));
        info
    });
    let before = *hsv;
    if let Some(at) = held(&response, rect) {
        hsv.h = at.x;
    }
    hsv.h = (hsv.h + arrows(ui, &response).x).clamp(0.0, 1.0);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.add(gradient(rect, 24, 1, |x, _| {
            Hsv {
                h: x,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }
            .solid()
        }));
        round_corners(painter, rect, STRIP * 0.5, p.elevated);
        frame(painter, p, &response, rect, 6);
        let at = pos2(rect.left() + hsv.h * rect.width(), rect.center().y);
        let pure = Hsv {
            s: 1.0,
            v: 1.0,
            a: 1.0,
            ..*hsv
        };
        handle(painter, at, 7.0, pure.solid());
    }
    response.on_hover_cursor(CursorIcon::PointingHand);
    *hsv != before
}

/// The colour from clear to whole across a strip, over the checkerboard.
fn opacity(ui: &mut Ui, p: &Palette, hsv: &mut Hsv) -> bool {
    let (rect, response) = strip(ui);
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Slider, enabled, "Opacity");
        info.value = Some(f64::from(hsv.a * 100.0));
        info
    });
    let before = *hsv;
    if let Some(at) = held(&response, rect) {
        hsv.a = at.x;
    }
    hsv.a = (hsv.a + arrows(ui, &response).x).clamp(0.0, 1.0);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        checker(painter, p, rect, 0);
        let solid = hsv.solid();
        painter.add(gradient(rect, 1, 1, |x, _| solid.gamma_multiply(x)));
        round_corners(painter, rect, STRIP * 0.5, p.elevated);
        frame(painter, p, &response, rect, 6);
        let at = pos2(rect.left() + hsv.a * rect.width(), rect.center().y);
        handle(painter, at, 7.0, shown(hsv.srgba()));
    }
    response.on_hover_cursor(CursorIcon::PointingHand);
    *hsv != before
}

/// Paints the swatch of `color` into `response`'s rect.
fn swatch(ui: &Ui, p: &Palette, response: &Response, color: [u8; 4], open: bool) {
    let rect = response.rect;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    if color[3] < 255 {
        checker(painter, p, rect, SWATCH_RADIUS);
    }
    painter.rect_filled(rect, SWATCH_RADIUS, shown(color));
    let enabled = ui.is_enabled();
    if open {
        painter.rect_stroke(
            rect,
            SWATCH_RADIUS,
            Stroke::new(2.0, p.fg),
            StrokeKind::Inside,
        );
    } else {
        // Neptune Video: the spec rims the swatch in `border`, which a
        // colour near the card's own ink swallows. The label ink at three
        // tenths, laid over the colour, stands off both it and the card.
        let edge = if enabled && response.hovered() {
            theme::tint(p.fg, 0.6)
        } else {
            p.fg.gamma_multiply(0.3)
        };
        painter.rect_stroke(
            rect,
            SWATCH_RADIUS,
            Stroke::new(1.0, edge),
            StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, SWATCH_RADIUS, *p);
    }
}

/// A colour well named `label`, for `color`. `alpha` offers the colour's
/// opacity, and `brand` the person's brand colours, in its popover. Changed
/// on every frame the colour changes in.
pub(super) fn well(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    color: &mut [u8; 4],
    alpha: bool,
    brand: &[[u8; 4]],
) -> Response {
    // Its own, where it is laid out: wells of one name in two cards (the
    // text's colour and the outline's) are two wells, and their pickers and
    // kept hues are apart.
    let id = ui.auto_id_with(("colour-well", label));
    let popover = id.with("popover");
    let open = egui::Popup::is_id_open(ui.ctx(), popover);
    let (_, mut response) = ui.allocate_exact_size(SWATCH, Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ColorButton, enabled, label));
    swatch(ui, p, &response, *color, open);

    // The hue is kept while the colour is the one picked, so a grey keeps
    // the hue it was dragged from.
    let kept: Option<([u8; 4], Hsv)> = ui.data(|data| data.get_temp(id));
    let mut hsv = match kept {
        Some((was, hsv)) if was == *color => hsv,
        _ => Hsv::of(*color),
    };
    let before = *color;
    let typed_id = id.with("typed");
    let surface = egui::Frame::NONE
        .fill(p.elevated)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(10)
        .shadow(p.popup_shadow())
        .inner_margin(12);
    egui::Popup::menu(&response)
        .id(popover)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(surface)
        .show(|ui| {
            ui.set_width(FIELD.x);
            ui.spacing_mut().item_spacing = vec2(6.0, 10.0);
            // A strip's target reaches 5 past what is drawn above and
            // under it, so the room after a part is set as it is laid out:
            // the gaps between what is drawn stay 10.
            ui.spacing_mut().item_spacing.y = 5.0;
            let mut moved = field(ui, p, &mut hsv);
            if alpha {
                ui.spacing_mut().item_spacing.y = 0.0;
            }
            moved |= hues(ui, p, &mut hsv);
            if alpha {
                ui.spacing_mut().item_spacing.y = 5.0;
                moved |= opacity(ui, p, &mut hsv);
            } else {
                hsv.a = f32::from(color[3]) / 255.0;
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            if moved {
                *color = hsv.srgba();
            }
            // The hex, as typed while it has the keyboard.
            let hex_id = id.with("hex");
            let focused = ui.memory(|memory| memory.has_focus(hex_id));
            let mut text: String = (ui.data(|data| data.get_temp(typed_id)))
                .filter(|_| focused)
                .unwrap_or_else(|| hex(*color, alpha));
            let typed = controls::text_field(ui, *p, hex_id, &mut text, "#RRGGBB", "Hex", FIELD.x);
            if typed.changed()
                && let Some(parsed) = parse(&text, if alpha { color[3] } else { 255 })
            {
                *color = parsed;
                hsv = Hsv::of(parsed);
            }
            // Read before the data is locked: a focus is read under the
            // same lock.
            let typing = typed.has_focus();
            ui.data_mut(|data| {
                if typing {
                    data.insert_temp(typed_id, text);
                } else {
                    data.remove::<String>(typed_id);
                }
            });
            if brand.is_empty() {
                return;
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Brand colours")
                        .font(theme::regular(11.5))
                        .color(p.secondary),
                )
                .selectable(false),
            );
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for swatch in brand {
                    let (rect, picked) = ui.allocate_exact_size(Vec2::splat(BRAND), Sense::click());
                    let name = format!("Brand colour {}", text_tab::hex(*swatch));
                    picked.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
                    let painter = ui.painter();
                    if swatch[3] < 255 {
                        checker(painter, p, rect, 5);
                    }
                    painter.rect_filled(rect, 5, shown(*swatch));
                    let ring = if *swatch == *color {
                        Stroke::new(1.5, p.fg)
                    } else if picked.hovered() {
                        Stroke::new(1.0, theme::tint(p.fg, 0.6))
                    } else {
                        Stroke::new(1.0, p.border)
                    };
                    painter.rect_stroke(rect, 5, ring, StrokeKind::Inside);
                    if picked.has_focus() {
                        controls::focus_ring(painter, rect, 5, *p);
                    }
                    if picked
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(&name)
                        .clicked()
                    {
                        *color = *swatch;
                        hsv = Hsv::of(*swatch);
                    }
                }
            });
        });
    if *color != before {
        response.mark_changed();
    }
    // What the picker holds is kept with the colour it made.
    if open || *color != before {
        ui.data_mut(|data| data.insert_temp(id, (*color, hsv)));
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A well for a colour held as linear red, green and blue from nought to
/// one, as a key colour is: the well shows and picks it in sRGB.
pub(super) fn linear_well(ui: &mut Ui, p: &Palette, label: &str, rgb: &mut [f32; 3]) -> Response {
    let shown = Color32::from(egui::Rgba::from_rgb(rgb[0], rgb[1], rgb[2]));
    let mut color = shown.to_srgba_unmultiplied();
    color[3] = 255;
    let response = well(ui, p, label, &mut color, false, &[]);
    if response.changed() {
        let picked = egui::Rgba::from(Color32::from_rgb(color[0], color[1], color[2]));
        *rgb = [picked.r(), picked.g(), picked.b()];
    }
    response
}
