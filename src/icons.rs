//! Tabler Icons 3.49.0, embedded as a small, dedicated native font subset.
//!
//! Glyphs retain Tabler's 24-point grid, rounded outline strokes and filled
//! transport controls. Widgets own labels, hit targets and state colors.
//! Source SVGs, mappings, regeneration and MIT notices live in
//! `assets/icons/tabler/`; no icon file is read or decoded during a frame.

use eframe::egui::{
    Color32, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect, Shape,
    emath::TSTransform,
};

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
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0 || color.a() == 0 || !painter.clip_rect().intersects(rect) {
        return;
    }
    let (variant, glyph) = tabler::mapping(icon);
    // Animated bounds must not create a new font raster for every fractional
    // size. Cache whole physical-pixel sizes and smoothly scale their meshes.
    let pixels_per_point = painter.pixels_per_point();
    let raster_size = (side * pixels_per_point).round().max(1.0) / pixels_per_point;
    let galley = painter.layout_no_wrap(
        glyph.to_string(),
        variant.font(raster_size),
        Color32::PLACEHOLDER,
    );
    let scale = side / raster_size;
    // Center the full 24-point cell, preserving Tabler's optical spacing.
    // Tight ink centering would shift asymmetric glyphs such as play.
    let translation = rect.center().to_vec2() - scale * galley.rect.center().to_vec2();
    let shape = if scale == 1.0 {
        Shape::galley(translation.to_pos2(), galley, color)
    } else {
        let mut shape = Shape::galley(Pos2::ZERO, galley, color);
        shape.transform(TSTransform::new(translation, scale));
        shape
    };
    painter.add(shape);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, LayerId, Vec2};

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
                    for side in [8.0, 14.0, 16.0, 24.0, 40.0] {
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
            assert_eq!(icon_shapes, tabler::ALL.len() * 5);
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            assert!(meshes.iter().any(|mesh| {
                matches!(&mesh.primitive, egui::epaint::Primitive::Mesh(mesh) if !mesh.is_empty())
            }));
        }
    }
}
