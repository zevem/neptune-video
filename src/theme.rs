//! The visual system: materials, text weights, metrics and egui defaults.
//!
//! The window is one quiet material. The library, viewer, inspector and
//! timeline sit in it as rounded surfaces separated by a gutter; depth comes
//! from elevation and hairlines, and the accent is kept for selection, the
//! playhead and the one primary action in view.
use eframe::egui::{self, Color32, FontFamily, FontId, Shadow, Stroke};
use serde::{Deserialize, Serialize};

pub fn color(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Font family names registered by [`install_fonts`].
pub const MEDIUM: &str = "Medium";
pub const SEMIBOLD: &str = "Semibold";

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
/// Timecode and numeric fields: digits that do not shift as they change.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Shared geometry. Radii are concentric: a panel's corner equals the corner
/// of what sits inside it plus the inset between them.
pub mod metrics {
    /// The window's corner: a panel's corner plus the gutter around it.
    pub const WINDOW_RADIUS: u8 = 18;
    pub const GUTTER: f32 = 6.0;
    pub const PANEL_RADIUS: u8 = 12;
    pub const TOOLBAR_HEIGHT: f32 = 46.0;
    pub const PANEL_HEADER: f32 = 36.0;
    pub const SHEET_RADIUS: u8 = 14;
    pub const MENU_RADIUS: u8 = 10;
    pub const CONTROL_RADIUS: u8 = 8;
    pub const CONTROL_HEIGHT: f32 = 28.0;
    /// Hit target of an icon-only control; the glyph inside is 16 points.
    pub const ICON_BUTTON: f32 = 28.0;
    pub const ICON: f32 = 16.0;
    pub const CLIP_RADIUS: u8 = 6;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Blue,
    Indigo,
    Purple,
    Pink,
    Orange,
    Green,
    Graphite,
}

impl Accent {
    pub const ALL: [(Accent, &'static str); 7] = [
        (Accent::Blue, "Blue"),
        (Accent::Indigo, "Indigo"),
        (Accent::Purple, "Purple"),
        (Accent::Pink, "Pink"),
        (Accent::Orange, "Orange"),
        (Accent::Green, "Green"),
        (Accent::Graphite, "Graphite"),
    ];

    pub fn color(self, dark: bool) -> Color32 {
        let (night, day) = match self {
            Accent::Blue => (0x0a84ff, 0x007aff),
            Accent::Indigo => (0x7d7aff, 0x5856d6),
            Accent::Purple => (0xbf5af2, 0xaf52de),
            Accent::Pink => (0xff4f79, 0xff2d55),
            Accent::Orange => (0xff9f0a, 0xf08a00),
            Accent::Green => (0x30d158, 0x28a745),
            Accent::Graphite => (0x98989f, 0x7c7c82),
        };
        color(if dark { night } else { day })
    }
}

/// What a clip or media item carries, for its colour and its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Video,
    Audio,
    Image,
    Text,
}

#[derive(Clone, Copy)]
pub struct Palette {
    /// Window material: toolbar, gutters and status line.
    pub chrome: Color32,
    /// Panels that sit in the window material.
    pub surface: Color32,
    /// The viewer's backdrop and the timeline's lanes: darker than a panel so
    /// that pictures and clips carry the weight.
    pub sunken: Color32,
    /// Sheets, popovers, menus and floating bars.
    pub elevated: Color32,
    /// Resting fill of fields, tracks and grouped rows.
    pub control: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// Hairline between regions that share a material.
    pub separator: Color32,
    /// Outline of floating surfaces and editable controls.
    pub border: Color32,
    pub fg: Color32,
    pub secondary: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub red: Color32,
    pub scrim: Color32,
    pub shadow: Color32,
    pub dark: bool,
}

/// Mixes `top` over `base`; both are treated as opaque.
pub fn mix(base: Color32, top: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        channel(base.r(), top.r()),
        channel(base.g(), top.g()),
        channel(base.b(), top.b()),
    )
}

/// A translucent tint of an opaque colour.
pub fn tint(color: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn luminance(color: Color32) -> f32 {
    0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32
}

impl Palette {
    pub fn new(appearance: Appearance, accent: Accent) -> Self {
        let dark = appearance == Appearance::Dark;
        let accent = accent.color(dark);
        let on_accent = if luminance(accent) > 170.0 {
            color(0x1d1d1f)
        } else {
            Color32::WHITE
        };
        let white = Color32::from_white_alpha;
        let black = Color32::from_black_alpha;
        if dark {
            Self {
                chrome: color(0x131315),
                surface: color(0x1c1c1f),
                sunken: color(0x0c0c0e),
                elevated: color(0x2a2a2e),
                control: white(16),
                hover: white(15),
                pressed: white(28),
                separator: white(18),
                border: white(30),
                fg: color(0xececf1),
                secondary: color(0xa0a0a8),
                muted: color(0x6d6d76),
                accent,
                on_accent,
                green: color(0x30d158),
                yellow: color(0xffd60a),
                red: color(0xff5a52),
                scrim: black(120),
                shadow: black(110),
                dark,
            }
        } else {
            Self {
                chrome: color(0xe6e6ea),
                surface: color(0xf7f7f9),
                sunken: color(0xdcdce1),
                elevated: color(0xffffff),
                control: black(13),
                hover: black(13),
                pressed: black(26),
                separator: black(22),
                border: black(34),
                fg: color(0x1d1d1f),
                secondary: color(0x5e5e66),
                muted: color(0x8e8e95),
                accent,
                on_accent,
                green: color(0x1f8f3d),
                yellow: color(0xa87c00),
                red: color(0xd92d20),
                scrim: black(70),
                shadow: black(46),
                dark,
            }
        }
    }

    /// The identity colour of a kind of media. Decoration only: a clip also
    /// says what it is with its icon and its name.
    pub fn tone(&self, tone: Tone) -> Color32 {
        color(match (tone, self.dark) {
            (Tone::Video, true) => 0x4f8cff,
            (Tone::Video, false) => 0x2f6fe0,
            (Tone::Audio, true) => 0x34c38f,
            (Tone::Audio, false) => 0x1e9c6c,
            (Tone::Image, true) => 0xb084f5,
            (Tone::Image, false) => 0x8a55d8,
            (Tone::Text, true) => 0xf2a65a,
            (Tone::Text, false) => 0xd07a1f,
        })
    }

    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.separator)
    }

    pub fn sheet_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: self.shadow,
        }
    }

    pub fn popup_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: self.shadow,
        }
    }
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut style = egui::Style {
        visuals: if p.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        },
        ..Default::default()
    };
    let visuals = &mut style.visuals;
    visuals.panel_fill = p.chrome;
    visuals.window_fill = p.elevated;
    visuals.extreme_bg_color = p.control;
    visuals.override_text_color = Some(p.fg);
    visuals.selection.bg_fill = tint(p.accent, if p.dark { 0.42 } else { 0.28 });
    visuals.selection.stroke = Stroke::new(1.5, p.accent);
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = egui::CornerRadius::same(metrics::SHEET_RADIUS);
    visuals.menu_corner_radius = egui::CornerRadius::same(metrics::MENU_RADIUS);
    visuals.window_shadow = p.sheet_shadow();
    visuals.popup_shadow = p.popup_shadow();
    visuals.widgets.noninteractive.bg_fill = p.elevated;
    visuals.widgets.noninteractive.bg_stroke = p.hairline();
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.secondary);
    for w in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = egui::CornerRadius::same(metrics::CONTROL_RADIUS);
        w.bg_stroke = Stroke::NONE;
        w.expansion = 0.0;
    }
    visuals.widgets.inactive.bg_fill = p.control;
    visuals.widgets.inactive.weak_bg_fill = p.control;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.secondary);
    visuals.widgets.hovered.bg_fill = p.hover;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.fg);
    visuals.widgets.active.bg_fill = p.pressed;
    visuals.widgets.active.weak_bg_fill = p.pressed;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.fg);
    visuals.widgets.open.bg_fill = p.pressed;
    visuals.widgets.open.weak_bg_fill = p.pressed;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, p.fg);
    visuals.text_cursor.stroke = Stroke::new(1.5, p.accent);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(5);
    style.spacing.interact_size.y = metrics::CONTROL_HEIGHT;
    style.spacing.scroll.floating = true;
    style.spacing.scroll.bar_width = 6.0;
    style.interaction.tooltip_delay = 0.45;
    style.animation_time = 0.12;
    for (text_style, size) in [
        (egui::TextStyle::Body, 13.0),
        (egui::TextStyle::Button, 13.0),
        (egui::TextStyle::Small, 11.0),
        (egui::TextStyle::Heading, 17.0),
        (egui::TextStyle::Monospace, 12.0),
    ] {
        let family = if text_style == egui::TextStyle::Monospace {
            FontFamily::Monospace
        } else {
            FontFamily::Proportional
        };
        style
            .text_styles
            .insert(text_style, FontId::new(size, family));
    }
    ctx.set_theme(if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_global_style(style);
}

/// Registers bundled Geist in three weights ahead of egui's own faces, which
/// remain as fallbacks for symbols and for the monospace family.
pub fn install_fonts(ctx: &egui::Context) {
    let mut definitions = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "Geist",
            include_bytes!("../assets/fonts/Geist-Regular.ttf").as_slice(),
        ),
        (
            "GeistMedium",
            include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice(),
        ),
        (
            "GeistSemiBold",
            include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice(),
        ),
    ] {
        definitions
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    let proportional = definitions
        .families
        .entry(FontFamily::Proportional)
        .or_default();
    proportional.insert(0, "Geist".into());
    // The other weights share the regular family's fallbacks, so emphasis
    // never changes which glyphs are available.
    let proportional = proportional.clone();
    for (family, face) in [(MEDIUM, "GeistMedium"), (SEMIBOLD, "GeistSemiBold")] {
        let mut fonts = proportional.clone();
        fonts[0] = face.into();
        definitions
            .families
            .insert(FontFamily::Name(family.into()), fonts);
    }
    crate::icons::register_fonts(&mut definitions);
    ctx.set_fonts(definitions);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accent_keeps_a_readable_label_in_both_appearances() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            for (accent, name) in Accent::ALL {
                let p = Palette::new(appearance, accent);
                let contrast = (luminance(p.accent) - luminance(p.on_accent)).abs();
                assert!(contrast > 70.0, "{appearance:?}/{name} label contrast");
            }
        }
    }

    #[test]
    fn media_tones_are_distinct_from_each_other() {
        let p = Palette::new(Appearance::Dark, Accent::default());
        let tones = [Tone::Video, Tone::Audio, Tone::Image, Tone::Text].map(|t| p.tone(t));
        for (i, a) in tones.iter().enumerate() {
            for b in &tones[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
