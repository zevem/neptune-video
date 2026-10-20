//! Tabler Icons 3.49.0, embedded as a small, dedicated native font subset.
//!
//! Glyphs retain Tabler's 24-point grid, rounded outline strokes and filled
//! transport controls. Widgets own labels, hit targets and state colors.
//! Source SVGs, mappings, regeneration and MIT notices live in
//! `assets/icons/tabler/`; no icon file is read or decoded during a frame.

use crate::ui::controls;
use eframe::egui::{
    Color32, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect,
    Response, Sense, Shape, StrokeKind, Ui, WidgetInfo, WidgetType,
    emath::{Rot2, TSTransform},
    epaint::{Galley, TextShape},
    pos2, vec2,
};

// The glyphs are in the subset before the areas that use them take them up.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Play,
    Pause,
    SkipStart,
    SkipEnd,
    StepBack,
    StepForward,
    Loop,
    Scissors,
    Trash,
    Undo,
    Redo,
    Copy,
    Clipboard,
    Plus,
    Close,
    Check,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Search,
    Folder,
    FolderPlus,
    Import,
    Save,
    Film,
    Image,
    Music,
    Text,
    Layers,
    Magnet,
    Flag,
    ZoomIn,
    ZoomOut,
    Fit,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Volume,
    VolumeOff,
    Sparkles,
    ArrowUp,
    ArrowDown,
    Settings,
    Sidebar,
    PanelRight,
    Grid,
    List,
    Warning,
    Link,
    Rotate,
    FlipHorizontal,
    FlipVertical,
    Pointer,
    File,
    History,
    Sort,
    Stop,
    Cube,
    Pipette,
    Diamond,
    Curve,
    FitValues,
    Transition,
    Captions,
    Export,
    /// A microphone: what is spoken.
    Speech,
    Minus,
    ArrowRight,
    DiamondFilled,
    Solo,
    LayoutFill,
    LayoutTopLeft,
    LayoutTopRight,
    LayoutBottomLeft,
    LayoutBottomRight,
    LayoutLeft,
    LayoutRight,
    /// "More": a menu of what does not fit.
    Dots,
    Keyboard,
    Palette,
    Pencil,
    /// A filled circle: the record disc and status dots.
    Disc,
    Headphones,
    Adjustments,
    Info,
    AlertCircle,
    CircleCheck,
    /// Filled, as the other transport controls are: a frame back.
    CaretLeft,
    CaretRight,
    ColorSwatch,
    Wand,
    FilePlus,
    Snowflake,
    ChartLine,
    MessagePlus,
    Sun,
    Moon,
    DeviceDesktop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Outline,
    Filled,
}

impl Variant {
    fn family(self) -> &'static str {
        match self {
            Self::Outline => "Tabler Outline",
            Self::Filled => "Tabler Filled",
        }
    }

    fn font(self, size: f32) -> FontId {
        FontId::new(size, FontFamily::Name(self.family().into()))
    }
}

mod tabler {
    use super::{Icon, Variant};
    include!("../assets/icons/tabler/glyphs.rs");
}

/// Add dedicated icon families without changing the text families' fallbacks.
pub fn register_fonts(definitions: &mut FontDefinitions) {
    for (variant, bytes) in [
        (
            Variant::Outline,
            include_bytes!("../assets/icons/tabler/outline.ttf").as_slice(),
        ),
        (
            Variant::Filled,
            include_bytes!("../assets/icons/tabler/filled.ttf").as_slice(),
        ),
    ] {
        let family = variant.family();
        definitions
            .font_data
            .insert(family.into(), FontData::from_static(bytes).into());
        definitions
            .families
            .insert(FontFamily::Name(family.into()), vec![family.into()]);
    }
}

/// Paint an icon inside its square design grid. The caller owns the hit area.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    paint_rotated(painter, rect, icon, color, 0.0);
}

/// Paint a glyph rotated clockwise around the design-cell center, in radians.
/// Rotation does not change the caller's layout bounds or hit target.
pub fn paint_rotated(painter: &Painter, rect: Rect, icon: Icon, color: Color32, angle: f32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite()
        || side <= 0.0
        || !angle.is_finite()
        || color.a() == 0
        || !painter.clip_rect().intersects(rect)
    {
        return;
    }
    let (variant, glyph) = tabler::mapping(icon);
    // Fractional/animated bounds reuse whole physical-pixel rasters. Mesh
    // scaling keeps movement smooth without growing the font atlas per frame.
    let pixels_per_point = painter.pixels_per_point();
    let raster_size = (side * pixels_per_point).round().max(1.0) / pixels_per_point;
    let galley = painter.layout_no_wrap(
        glyph.to_string(),
        variant.font(raster_size),
        Color32::PLACEHOLDER,
    );
    let scale = side / raster_size;
    // Preserve Tabler's full 24-point cell and optical offsets, not tight ink
    // centering. Rotate around that same cell for spinners and disclosure.
    let cell = cell_center(&galley, raster_size);
    let rotated = Rot2::from_angle(angle) * cell.to_vec2();
    let shape = if scale == 1.0 {
        Shape::Text(TextShape::new(rect.center() - rotated, galley, color).with_angle(angle))
    } else {
        let mut shape =
            Shape::Text(TextShape::new(Pos2::ZERO - rotated, galley, color).with_angle(angle));
        shape.transform(TSTransform::new(rect.center().to_vec2(), scale));
        shape
    };
    painter.add(shape);
}

/// The centre of a laid-out glyph's design cell, relative to its galley.
///
/// Tabler's em square is the 24-point cell: 0.9 em above the baseline and
/// 0.1 em below it. The font also declares a typographic line gap, which egui
/// adds to the row, so the galley's bounds are taller than the cell and
/// centring them would lift every glyph.
pub(crate) fn cell_center(galley: &Galley, size: f32) -> Pos2 {
    let Some((row, glyph)) = galley
        .rows
        .first()
        .and_then(|row| Some((row, row.glyphs.first()?)))
    else {
        return galley.rect.center();
    };
    let baseline = row.pos + glyph.pos.to_vec2();
    pos2(
        baseline.x + glyph.advance_width * 0.5,
        baseline.y - size * (CELL_ASCENT - 0.5),
    )
}

/// The share of Tabler's em square above the baseline (900 of 1,000 units).
const CELL_ASCENT: f32 = 0.9;

/// A compact, keyboard-focusable icon button with a rounded hover surface.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    button_with_hint(ui, icon, tooltip, "")
}

/// As [`button`], with a keyboard shortcut shown beside the tooltip. The
/// accessible name stays the plain label.
pub fn button_with_hint(ui: &mut Ui, icon: Icon, label: &str, shortcut: &str) -> Response {
    let (_, rect) = ui.allocate_space(vec2(28.0, 28.0));
    let response = ui.interact(rect, ui.id().with(("icon-button", label)), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    paint_button(ui, &response, icon);
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    hint(response, label, shortcut)
}

/// Paints an icon button's surface, focus and glyph over a response the
/// caller made, so a control that keeps an identity of its own looks the same.
// Neptune Video: split out of `button_with_hint`, whose body it is.
pub fn paint_button(ui: &Ui, response: &Response, icon: Icon) {
    let rect = response.rect;
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(response);
        let surface = rect.shrink(1.0);
        if response.hovered() || response.is_pointer_button_down_on() {
            ui.painter().rect_filled(surface, 7, visuals.weak_bg_fill);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                surface,
                7,
                ui.visuals().selection.stroke,
                StrokeKind::Inside,
            );
        }
        // Pressing settles the glyph slightly, like a physical key.
        let inset = if response.is_pointer_button_down_on() {
            6.5
        } else {
            6.0
        };
        paint(
            ui.painter(),
            rect.shrink(inset),
            icon,
            visuals.fg_stroke.color,
        );
    }
}

/// The tooltip of a control: its label, and its shortcut set as keycaps
/// three spaces after it.
// Neptune Video: Terminal writes the shortcut as text; the spec draws keys.
pub fn hint(response: Response, label: &str, shortcut: &str) -> Response {
    if shortcut.is_empty() {
        return response.on_hover_text(label);
    }
    response.on_hover_ui(|ui| {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add_space(3.0);
            let ink = ui.visuals().text_color();
            // The caps are as `keycaps` sets them: each at least 20 wide,
            // 3 apart.
            let width: f32 = controls::chord_keys(shortcut)
                .into_iter()
                .map(|key| {
                    let galley = ui.painter().layout_no_wrap(
                        key.to_owned(),
                        crate::theme::medium(10.5),
                        ink,
                    );
                    (galley.size().x + 10.0).max(20.0) + 3.0
                })
                .sum();
            let (rect, _) = ui.allocate_exact_size(vec2(width - 3.0, 18.0), Sense::hover());
            controls::keycaps(ui.painter(), rect.right_center(), shortcut, ink);
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, LayerId, Vec2};
    // Spec §3.8 (F7): glyphs are placed by Tabler's design cell, not by the
    // galley's line box.

    #[test]
    fn every_tabler_icon_is_present_in_its_dedicated_font() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            for icon in tabler::ALL {
                let (variant, glyph) = tabler::mapping(icon);
                assert!(
                    ctx.fonts_mut(|fonts| fonts.has_glyph(&variant.font(16.0), glyph)),
                    "{icon:?} is missing from {variant:?}"
                );
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn all_icons_draw_at_small_and_large_sizes_on_both_display_scales() {
        for pixels_per_point in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx.set_pixels_per_point(pixels_per_point);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx();
                let painter = ctx.layer_painter(LayerId::background());
                for icon in tabler::ALL {
                    for side in [8.0, 14.0, 16.0, 16.25, 24.0, 40.0] {
                        let rect = Rect::from_center_size(Pos2::new(64.0, 64.0), Vec2::splat(side));
                        paint(&painter, rect, icon, Color32::WHITE);
                    }
                }
            });
            output.textures_delta.clear();
            let icon_shapes = output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, Shape::Text(_)))
                .count();
            assert_eq!(icon_shapes, tabler::ALL.len() * 6);
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            assert!(meshes.iter().any(|mesh| {
                matches!(&mesh.primitive, egui::epaint::Primitive::Mesh(mesh) if !mesh.is_empty())
            }));
        }
    }

    /// The coverage-weighted centre of a painted glyph's ink, in points.
    fn ink_center(pixels_per_point: f32, rect: Rect, icon: Icon, angle: f32) -> Pos2 {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.set_pixels_per_point(pixels_per_point);
        let mut center = Pos2::ZERO;
        // The second frame paints with the glyph already in the font atlas.
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint_rotated(ui.painter(), rect, icon, Color32::WHITE, angle);
            });
            output.textures_delta.clear();
            let atlas = ctx.fonts(|fonts| fonts.image());
            let [width, height] = atlas.size.map(|size| size as f32);
            let (mut sum, mut weight) = (vec2(0.0, 0.0), 0.0);
            for primitive in ctx.tessellate(output.shapes, pixels_per_point) {
                let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                    continue;
                };
                // Each glyph is a quad: top-left, top-right, bottom-left,
                // bottom-right, possibly rotated.
                for quad in mesh.vertices.chunks_exact(4) {
                    let (u0, v0) = (quad[0].uv.x * width, quad[0].uv.y * height);
                    let (u1, v1) = (quad[3].uv.x * width, quad[3].uv.y * height);
                    let across = quad[1].pos - quad[0].pos;
                    let down = quad[2].pos - quad[0].pos;
                    for v in v0.round() as usize..v1.round() as usize {
                        for u in u0.round() as usize..u1.round() as usize {
                            let alpha = f32::from(atlas[(u, v)].a()) / 255.0;
                            let s = (u as f32 + 0.5 - u0) / (u1 - u0);
                            let t = (v as f32 + 0.5 - v0) / (v1 - v0);
                            sum += (quad[0].pos + across * s + down * t).to_vec2() * alpha;
                            weight += alpha;
                        }
                    }
                }
            }
            center = (sum / weight).to_pos2();
        }
        center
    }

    #[test]
    fn glyphs_stay_centered_in_their_design_cell_at_every_scale_and_angle() {
        // Tabler's plus is symmetric about its cell centre, so its ink centre
        // is the cell centre at any rotation. Text lands on whole physical
        // pixels, leaving under one pixel of error; centring the galley's
        // line box instead lifted glyphs by 1.6 pixels at 150 %.
        for pixels_per_point in [1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
            for origin in [pos2(10.0, 10.0), pos2(10.5, 11.25)] {
                for side in [16.0, 15.5, 14.0, 16.25] {
                    for angle in [
                        0.0,
                        std::f32::consts::FRAC_PI_2,
                        std::f32::consts::TAU / 3.0,
                    ] {
                        let rect = Rect::from_min_size(origin, vec2(side, side));
                        let offset =
                            ink_center(pixels_per_point, rect, Icon::Plus, angle) - rect.center();
                        let limit = 0.75 / pixels_per_point;
                        assert!(
                            offset.x.abs() <= limit && offset.y.abs() <= limit,
                            "glyph off centre by {offset:?} at {pixels_per_point}x, \
                             {angle} rad in {rect:?}"
                        );
                    }
                }
            }
        }
    }
}
