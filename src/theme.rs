//! The visual system: materials, text weights, metrics and egui defaults.
//!
//! Neptune Video shares Neptune Terminal's window, materials and tokens. The
//! window is one quiet material; the timeline sits on the darker content
//! surface and the viewer on a stage that stays dark in both appearances, so
//! a picture is always judged against a neutral surround. Depth comes from
//! elevation and hairlines, and the accent is kept for focus, selection and
//! the one primary action in view.

use eframe::egui::{self, Color32, FontFamily, FontId, Shadow, Stroke};
use serde::{Deserialize, Serialize};

pub fn color(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Font family names registered by [`install_fonts`].
pub const MEDIUM: &str = "Medium";
pub const SEMIBOLD: &str = "Semibold";

/// Body copy, values and menu rows.
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
/// Names, labels and buttons.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
/// Sheet titles.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
/// Timecode and readouts: JetBrains Mono, whose digits do not shift as they
/// change.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Shared geometry, with Neptune Terminal's names and values. Radii are
/// concentric: the window corner equals the pane corner plus the gutter
/// between them.
pub mod metrics {
    /// The window's corner: the pane corner and the gutter around it.
    pub const WINDOW_RADIUS: u8 = 16;
    pub const GUTTER: f32 = 6.0;
    /// Pane wells and cards.
    pub const PANE_RADIUS: u8 = 10;
    /// The toolbar, which is also the window's title bar.
    pub const TOOLBAR_HEIGHT: f32 = 44.0;
    /// Library and inspector tabs, the timeline's strip, the source header.
    pub const PANE_HEADER: f32 = 34.0;
    /// The transport and the library's footer.
    pub const PANE_FOOTER: f32 = 44.0;
    pub const SHEET_RADIUS: u8 = 14;
    /// Menus, tooltips and popovers.
    pub const MENU_RADIUS: u8 = 10;
    pub const TOAST_RADIUS: u8 = 12;
    pub const CONTROL_RADIUS: u8 = 8;
    pub const ROW_RADIUS: u8 = 8;
    /// Buttons, text fields and selects.
    pub const CONTROL_HEIGHT: f32 = 30.0;
    /// Segmented controls, steppers, menu rows and the library's search.
    pub const COMPACT_HEIGHT: f32 = 28.0;
    /// Buttons, number fields and selects inside an inspector row.
    pub const INSPECTOR_CONTROL: f32 = 26.0;
    /// Hit target of an icon-only control: a 26-point surface around a
    /// 16-point glyph.
    pub const ICON_BUTTON: f32 = 28.0;
    pub const ICON: f32 = 16.0;
    /// Head cells, diamonds and card actions: a 22-point target around a
    /// 14-point glyph.
    pub const SMALL_BUTTON: f32 = 22.0;
    pub const INSPECTOR_ROW: f32 = 32.0;
    /// The keyframe controls at the end of a keyed row: a step back, the
    /// diamond and a step on, 18 + 2 + 22 + 2 + 18.
    pub const KEY_COLUMN: f32 = 62.0;
    pub const PREFERENCES_ROW: f32 = 42.0;
    /// Both side panels fit beside the viewer from this window width.
    pub const SIDEBAR_MIN_WINDOW: f32 = 820.0;
    pub const CLIP_RADIUS: u8 = 6;
    /// Room the picture keeps inside the stage on every side.
    pub const STAGE_INSET: f32 = 18.0;
}

/// How long eased state changes take, in seconds. Editing feedback (the
/// playhead, drags, trims, zoom and selection) never eases, and with Reduce
/// motion on nothing does.
pub mod motion {
    /// Hover fills, ink and glyph reveals.
    pub const HOVER: f32 = 0.12;
    pub const SWITCH: f32 = 0.16;
    pub const SEGMENTED: f32 = 0.14;
    /// A side panel showing or hiding.
    pub const PANEL: f32 = 0.16;
    /// A sheet and its scrim fading in.
    pub const SHEET: f32 = 0.16;
    pub const TOAST: f32 = 0.12;
    /// Play turning into pause.
    pub const PLAY_GLYPH: f32 = 0.12;
    pub const ATTENTION: f32 = 0.16;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Whichever the system shows.
    System,
}

impl Appearance {
    /// The appearance to paint, given the system's when it is known. A system
    /// that does not say is taken to be dark.
    pub fn resolve(self, system: Option<egui::Theme>) -> Appearance {
        match (self, system) {
            (Appearance::System, Some(egui::Theme::Light)) => Appearance::Light,
            (Appearance::System, _) => Appearance::Dark,
            (chosen, _) => chosen,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Blue,
    Indigo,
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Graphite,
}

impl Accent {
    pub const ALL: [(Accent, &'static str); 9] = [
        (Accent::Blue, "Blue"),
        (Accent::Indigo, "Indigo"),
        (Accent::Purple, "Purple"),
        (Accent::Pink, "Pink"),
        (Accent::Red, "Red"),
        (Accent::Orange, "Orange"),
        (Accent::Yellow, "Yellow"),
        (Accent::Green, "Green"),
        (Accent::Graphite, "Graphite"),
    ];

    pub fn color(self, dark: bool) -> Color32 {
        let (night, day) = match self {
            Accent::Blue => (0x0a84ff, 0x007aff),
            Accent::Indigo => (0x7d7aff, 0x5856d6),
            Accent::Purple => (0xbf5af2, 0xaf52de),
            Accent::Pink => (0xff4f79, 0xff2d55),
            Accent::Red => (0xff5a52, 0xff3b30),
            Accent::Orange => (0xff9f0a, 0xf08a00),
            Accent::Yellow => (0xffd60a, 0xe0a800),
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

/// One channel of an RGB curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Red,
    Green,
    Blue,
}

#[derive(Clone, Copy)]
pub struct Palette {
    /// Content surface: the timeline and the wells of curves and fields.
    pub bg: Color32,
    /// Window material shared by the toolbar, the gutters, the library and
    /// the inspector.
    pub chrome: Color32,
    /// Sheets, popovers, menus and floating bars.
    pub elevated: Color32,
    /// Resting fill of fields, cards and secondary buttons.
    pub control: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// Hairline between regions that share a material.
    pub separator: Color32,
    /// Outline of floating surfaces and of the window.
    pub border: Color32,
    pub fg: Color32,
    pub secondary: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub red: Color32,
    /// What needs the person: an agent waiting, a count, a file changed on
    /// disk.
    pub attention: Color32,
    /// Unused here; kept so the palette has Terminal's shape.
    pub merged: Color32,
    pub scrim: Color32,
    pub shadow: Color32,
    pub dark: bool,
    /// The accent chosen, for the tokens that step aside from it.
    accent_choice: Accent,
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

/// Perceived brightness, nought to 255.
#[cfg(test)]
fn luminance(color: Color32) -> f32 {
    0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32
}

/// Relative luminance of opaque sRGB colors, used only for readability.
fn relative_luminance(c: Color32) -> f32 {
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// The WCAG contrast ratio of two opaque colours.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let a = relative_luminance(a);
    let b = relative_luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

impl Palette {
    /// The palette of an appearance. `System` is resolved by the app before
    /// it gets here; one that is not is painted dark.
    pub fn new(appearance: Appearance, accent: Accent) -> Self {
        let dark = appearance != Appearance::Light;
        // Attention is amber, and stays apart from an amber focus accent.
        let attention = if accent == Accent::Orange {
            Accent::Yellow
        } else {
            Accent::Orange
        }
        .color(dark);
        let accent_choice = accent;
        let accent = accent.color(dark);
        // Dark ink on the bright accents, where white would be faint:
        // Yellow in both appearances, Orange and Green in the dark one.
        // Terminal reckons this from the accent's brightness; the light
        // Yellow and the dark Green fall on the wrong side of its line.
        let dark_ink = match accent_choice {
            Accent::Yellow => true,
            Accent::Orange | Accent::Green => dark,
            _ => false,
        };
        let on_accent = if dark_ink {
            color(0x1d1d1f)
        } else {
            Color32::WHITE
        };
        let white = Color32::from_white_alpha;
        let black = Color32::from_black_alpha;
        let mut p = Self {
            bg: color(0x101012),
            chrome: color(0x1c1c1f),
            elevated: color(0x29292d),
            control: white(16),
            hover: white(15),
            pressed: white(28),
            separator: white(20),
            border: white(30),
            fg: color(0xececf1),
            secondary: color(0xa0a0a8),
            muted: color(0x6d6d76),
            accent,
            on_accent,
            green: color(0x30d158),
            yellow: color(0xffd60a),
            red: color(0xff5a52),
            attention,
            merged: color(0xc792f6),
            scrim: black(120),
            shadow: black(110),
            dark,
            accent_choice,
        };
        if !dark {
            p.bg = color(0xffffff);
            p.chrome = color(0xececef);
            p.elevated = color(0xffffff);
            p.control = black(13);
            p.hover = black(13);
            p.pressed = black(26);
            p.separator = black(22);
            p.border = black(34);
            p.fg = color(0x1d1d1f);
            p.secondary = color(0x5e5e66);
            p.muted = color(0x8e8e95);
            p.green = color(0x28a745);
            p.yellow = color(0xe0a800);
            p.red = color(0xe5372d);
            p.merged = color(0x9340c8);
            p.scrim = black(70);
            p.shadow = black(46);
        }
        p
    }

    /// What is drawn inside the viewer: always the dark palette, so the
    /// transport, the timecode and the overlays read on the stage in either
    /// appearance.
    pub fn stage_palette(accent: Accent) -> Self {
        Self::new(Appearance::Dark, accent)
    }

    /// Label ink on a filled destructive control: white, unless the red is
    /// too light to carry it.
    pub fn on_red(&self) -> Color32 {
        if contrast(Color32::WHITE, self.red) >= 3.0 {
            Color32::WHITE
        } else {
            color(0x1d1d1f)
        }
    }

    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.separator)
    }

    /// Selected text and the fill behind a selection.
    pub fn selection_fill(&self) -> Color32 {
        tint(self.accent, if self.dark { 0.42 } else { 0.28 })
    }

    /// Soft elevation for sheets.
    pub fn sheet_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: self.shadow,
        }
    }

    /// Lighter elevation for menus, tooltips, toasts and floating bars.
    pub fn popup_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: self.shadow.gamma_multiply(0.7),
        }
    }

    /// `c` as ink on a light surface. Yellow and amber are too faint on
    /// white for a glyph or a thin line, so there they are darkened; in the
    /// dark appearance, and for colours that already read, `c` is kept.
    pub fn legible(&self, c: Color32) -> Color32 {
        if self.dark || contrast(c, Color32::WHITE) >= 3.0 {
            c
        } else {
            mix(c, Color32::BLACK, 0.4)
        }
    }

    /// The identity colour of a kind of media. Decoration only: a clip also
    /// says what it is with its icon and its name.
    pub fn tone(&self, tone: Tone) -> Color32 {
        color(match (tone, self.dark) {
            (Tone::Video, true) => 0x8e9cc7,
            (Tone::Video, false) => 0x55658f,
            (Tone::Audio, true) => 0x34c38f,
            (Tone::Audio, false) => 0x1e9c6c,
            (Tone::Image, true) => 0xb084f5,
            (Tone::Image, false) => 0x8a55d8,
            (Tone::Text, true) => 0xf2a65a,
            (Tone::Text, false) => 0xd07a1f,
        })
    }

    /// The viewer's surround, dark in both appearances; also the wells
    /// posters and title previews sit in.
    pub fn stage(&self) -> Color32 {
        color(0x101012)
    }

    /// A tile's body where no picture covers it.
    pub fn tile_body(&self, tone: Tone) -> Color32 {
        if self.dark {
            mix(color(0x1c1c1f), self.tone(tone), 0.34)
        } else {
            mix(Color32::WHITE, self.tone(tone), 0.30)
        }
    }

    /// A selected tile's body.
    pub fn tile_lift(&self, tone: Tone) -> Color32 {
        if self.dark {
            mix(color(0x1c1c1f), self.tone(tone), 0.46)
        } else {
            mix(Color32::WHITE, self.tone(tone), 0.42)
        }
    }

    /// The line that keeps abutting tiles apart.
    pub fn tile_edge(&self) -> Color32 {
        if self.dark {
            Color32::from_white_alpha(18)
        } else {
            Color32::from_black_alpha(22)
        }
    }

    /// The band a tile's name sits on over a picture, under white ink.
    pub fn tile_band(&self) -> Color32 {
        Color32::from_black_alpha(110)
    }

    /// A contrast line just inside a selection ring.
    pub fn keyline(&self) -> Color32 {
        if self.dark {
            Color32::from_black_alpha(140)
        } else {
            Color32::from_white_alpha(200)
        }
    }

    /// A track's row.
    pub fn lane(&self) -> Color32 {
        if self.dark {
            Color32::from_white_alpha(7)
        } else {
            Color32::from_black_alpha(8)
        }
    }

    /// The target track's row, and the row a drag would drop on.
    pub fn lane_target(&self) -> Color32 {
        if self.dark {
            Color32::from_white_alpha(12)
        } else {
            Color32::from_black_alpha(12)
        }
    }

    /// The hatching over a locked track.
    pub fn lane_hatch(&self) -> Color32 {
        tint(self.fg, if self.dark { 0.05 } else { 0.06 })
    }

    pub fn ruler(&self) -> Color32 {
        tint(self.fg, if self.dark { 0.03 } else { 0.04 })
    }

    /// The playhead is ink, not the accent: it is where, not what is chosen.
    pub fn playhead(&self) -> Color32 {
        self.fg
    }

    /// The halo either side of the playhead that keeps it clear of pictures.
    pub fn playhead_halo(&self) -> Color32 {
        if self.dark {
            Color32::from_black_alpha(80)
        } else {
            Color32::from_white_alpha(160)
        }
    }

    /// Snap guides: yellow, unless yellow is the accent.
    pub fn snap(&self) -> Color32 {
        if self.accent_choice == Accent::Yellow {
            self.fg
        } else {
            self.legible(self.yellow)
        }
    }

    pub fn wave(&self) -> Color32 {
        let audio = self.tone(Tone::Audio);
        if self.dark {
            mix(audio, Color32::WHITE, 0.2)
        } else {
            mix(audio, Color32::BLACK, 0.1)
        }
    }

    /// Waveform columns that reach full scale.
    pub fn clip_peak(&self) -> Color32 {
        self.red
    }

    pub fn volume_line(&self) -> Color32 {
        tint(self.fg, 0.85)
    }

    /// What a fade takes away.
    pub fn fade_shade(&self) -> Color32 {
        Color32::from_black_alpha(if self.dark { 90 } else { 40 })
    }

    /// A fade's edge and beat ticks.
    pub fn fade_curve(&self) -> Color32 {
        tint(self.fg, 0.5)
    }

    pub fn key(&self) -> Color32 {
        self.fg
    }

    pub fn key_selected(&self) -> Color32 {
        self.accent
    }

    /// Identities of the graph editor's parameters, in turn. Never a status.
    pub fn curve(&self, index: usize) -> Color32 {
        const NIGHT: [u32; 6] = [0x8e9cc7, 0xf2a65a, 0xb084f5, 0x5fd3c4, 0xff7eb6, 0xc9c96a];
        const DAY: [u32; 6] = [0x55658f, 0xc26a12, 0x7e48cf, 0x13897c, 0xd63a7f, 0x7f7f1f];
        color(if self.dark { NIGHT } else { DAY }[index % 6])
    }

    /// An RGB curve's channel.
    pub fn channel(&self, channel: Channel) -> Color32 {
        color(match (channel, self.dark) {
            (Channel::Red, true) => 0xff6159,
            (Channel::Green, true) => 0x3ccf6a,
            (Channel::Blue, true) => 0x4c9dff,
            (Channel::Red, false) => 0xd93a32,
            (Channel::Green, false) => 0x1f9a4a,
            (Channel::Blue, false) => 0x2f6fe0,
        })
    }

    /// The two squares of the checkerboard under a translucent colour.
    pub fn checker(&self) -> (Color32, Color32) {
        if self.dark {
            (color(0x3a3a3e), color(0x2a2a2e))
        } else {
            (Color32::WHITE, color(0xe5e5e8))
        }
    }

    /// The keyline round the picture: the stage palette's separator, since
    /// the stage is dark in both appearances.
    pub fn canvas_edge(&self) -> Color32 {
        Color32::from_white_alpha(20)
    }

    /// A canvas transform handle: its fill and its rim.
    pub fn handle(&self) -> (Color32, Color32) {
        (Color32::WHITE, Color32::from_black_alpha(102))
    }

    /// Outside the accent frame round the canvas.
    pub fn frame_halo(&self) -> Color32 {
        Color32::from_black_alpha(102)
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
    visuals.selection.bg_fill = p.selection_fill();
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
    style.spacing.interact_size.y = metrics::COMPACT_HEIGHT;
    style.spacing.scroll.floating = true;
    style.spacing.scroll.bar_width = 6.0;
    style.interaction.tooltip_delay = 0.45;
    // Reduce motion settles every eased change at once.
    style.animation_time = if crate::ui::kit::reduced_motion(ctx) {
        0.0
    } else {
        motion::HOVER
    };
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

/// Registers bundled Geist in three weights and JetBrains Mono ahead of
/// egui's own faces, which remain as fallbacks for symbols.
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
        (
            "JetBrains",
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
        ),
    ] {
        definitions
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    // The family Neptune's apps share for digits that must not jitter.
    definitions
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains".into());
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
        // Spec §3.3: nine accents, Red and Yellow among them.
        assert_eq!(Accent::ALL.len(), 9);
        for appearance in [Appearance::Dark, Appearance::Light] {
            for (accent, name) in Accent::ALL {
                let p = Palette::new(appearance, accent);
                let contrast = (luminance(p.accent) - luminance(p.on_accent)).abs();
                assert!(contrast > 70.0, "{appearance:?}/{name} label contrast");
                // Spec §3.3: the ink each accent is given.
                let dark_ink = match accent {
                    Accent::Yellow => true,
                    Accent::Orange | Accent::Green => appearance == Appearance::Dark,
                    _ => false,
                };
                let ink = if dark_ink {
                    color(0x1d1d1f)
                } else {
                    Color32::WHITE
                };
                assert_eq!(p.on_accent, ink, "{appearance:?}/{name} label ink");
                // A count or a waiting agent is never taken for focus.
                assert_ne!(p.attention, p.accent, "{appearance:?}/{name}");
                assert_eq!(p.on_red(), Color32::WHITE);
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
        // Spec §3.2 (F4): video is steel, so a clip is not read as selected
        // under the Blue accent.
        assert_eq!(p.tone(Tone::Video), color(0x8e9cc7));
        let light = Palette::new(Appearance::Light, Accent::default());
        assert_eq!(light.tone(Tone::Video), color(0x55658f));
    }

    #[test]
    fn yellow_and_attention_are_legible_as_ink_on_white() {
        for accent in [Accent::Blue, Accent::Orange] {
            let p = Palette::new(Appearance::Light, accent);
            for ink in [p.yellow, p.attention] {
                let legible = p.legible(ink);
                assert!(
                    contrast(legible, Color32::WHITE) >= 3.0,
                    "{ink:?} as {legible:?} on white"
                );
            }
            // Colours that already read are kept.
            assert_eq!(p.legible(p.red), p.red);
            assert_eq!(p.legible(p.green), p.green);
        }
        let p = Palette::new(Appearance::Light, Accent::Blue);
        assert_eq!(p.legible(p.yellow), color(0x866500));
        assert_eq!(p.legible(p.attention), color(0x905300));
        assert_eq!(p.snap(), color(0x866500));
        // The dark appearance and the stage keep their colours.
        let dark = Palette::new(Appearance::Dark, Accent::Blue);
        assert_eq!(dark.legible(dark.yellow), dark.yellow);
        assert_eq!(Palette::new(Appearance::Light, Accent::Yellow).snap(), p.fg);
    }

    #[test]
    fn the_system_appearance_resolves_to_what_the_system_shows() {
        use egui::Theme;
        assert_eq!(Appearance::System.resolve(None), Appearance::Dark);
        assert_eq!(
            Appearance::System.resolve(Some(Theme::Dark)),
            Appearance::Dark
        );
        assert_eq!(
            Appearance::System.resolve(Some(Theme::Light)),
            Appearance::Light
        );
        // A chosen appearance is kept whatever the system shows.
        for system in [None, Some(Theme::Dark), Some(Theme::Light)] {
            assert_eq!(Appearance::Dark.resolve(system), Appearance::Dark);
            assert_eq!(Appearance::Light.resolve(system), Appearance::Light);
        }
    }

    #[test]
    fn the_stage_stays_dark_in_both_appearances() {
        let light = Palette::new(Appearance::Light, Accent::Blue);
        assert_eq!(light.stage(), color(0x101012));
        let stage = Palette::stage_palette(Accent::Blue);
        assert!(stage.dark);
        assert_eq!(stage.bg, color(0x101012));
        // The keyline round the picture is drawn on the stage, so it is the
        // stage palette's in either appearance.
        let dark = Palette::new(Appearance::Dark, Accent::Blue);
        assert_eq!(light.canvas_edge(), stage.separator);
        assert_eq!(dark.canvas_edge(), stage.separator);
    }
}
