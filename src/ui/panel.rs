//! What Neptune Terminal's side panel shares with the panels here: the strip
//! of tabs at a panel's top, a toolbar control that stays pressed while its
//! panel is open, and the dot that says something in it needs the person.
//!
//! Ported from Terminal's `ui/panel.rs`, where these serve one panel's own
//! tabs; here a strip is given its tabs, so the library and the inspector can
//! share it. Where Neptune Video departs, the code says so.
#![allow(dead_code)] // Preserve the full shared Neptune component vocabulary.

use super::controls::{animate, elided, focus_ring, galley_at};
use crate::{
    icons::{self, Icon},
    theme::{self, Palette},
};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

/// The strip of tabs above a panel's content.
pub const TABS: f32 = 34.0;
/// A tab's icon, where it stands for its name.
const ICON: f32 = 14.0;

/// One tab of a strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tab<'a> {
    /// Its name, which is also its accessible name.
    pub label: &'a str,
    /// What stands for it where the strip has no room for names.
    pub icon: Icon,
    /// What waits in it for the person, counted on the tab.
    pub waiting: usize,
}

/// How the names of the tabs are set in the strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub font: f32,
    /// Clear space inside a tab, both sides together.
    pub padding: f32,
    pub gap: f32,
    /// Each tab is its icon, named under the pointer.
    pub icons: bool,
}

impl Fit {
    pub const ROOMY: Self = Self {
        font: 12.0,
        padding: 16.0,
        gap: 4.0,
        icons: false,
    };
    /// For a panel short of room, where the names are set closer.
    pub const TIGHT: Self = Self {
        font: 11.5,
        padding: 8.0,
        gap: 2.0,
        icons: false,
    };
    /// For a strip with no room for names.
    // Neptune Video: no step cuts names to a short form; they give way to
    // their icons at once, in equal cells.
    pub const ICONS: Self = Self {
        font: 11.5,
        padding: 12.0,
        gap: 2.0,
        icons: true,
    };
}

/// A dot in the attention colour over a control, cut out of what is beneath.
pub fn attention_dot(ui: &Ui, p: Palette, response: &Response, on: bool, pressed: bool) {
    let dot = animate(
        ui.ctx(),
        response.id.with("attention"),
        on,
        theme::motion::ATTENTION,
    );
    if dot <= 0.0 {
        return;
    }
    let beneath = if pressed || response.is_pointer_button_down_on() {
        p.pressed
    } else if response.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    let centre = response.rect.center() + vec2(7.0, -6.0);
    let painter = ui.painter();
    painter.circle_filled(centre, 5.0 * dot, p.chrome);
    painter.circle_filled(centre, 5.0 * dot, beneath);
    icons::paint(
        painter,
        Rect::from_center_size(centre, vec2(8.4, 8.4) * dot),
        Icon::Disc,
        p.attention,
    );
}

/// A toolbar control for a panel. It stays pressed while the panel is open
/// and carries a dot while something out of view waits for the person.
// Neptune Video: given its glyph and name, for each panel that has one.
pub fn toggle(
    ui: &mut Ui,
    p: Palette,
    icon: Icon,
    label: &str,
    hint: &str,
    open: bool,
    waiting: bool,
) -> Response {
    let surface = ui.painter().add(egui::Shape::Noop);
    let response = icons::button_with_hint(ui, icon, label, hint);
    // Neptune Video: it says whether it is on, as a check box does.
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), open, label));
    if open {
        ui.painter().set(
            surface,
            egui::Shape::rect_filled(response.rect.shrink(1.0), 7, p.pressed),
        );
    }
    attention_dot(ui, p, &response, waiting, open);
    response
}

/// One tab of the strip. Returns whether it was chosen.
fn tab(
    ui: &mut Ui,
    rect: Rect,
    p: Palette,
    item: Tab,
    selected: bool,
    role: WidgetType,
    fit: Fit,
) -> bool {
    let response = ui.interact(
        rect,
        ui.id().with(("panel-tab", item.label)),
        Sense::click(),
    );
    response.widget_info(|| WidgetInfo::selected(role, ui.is_enabled(), selected, item.label));
    let fill = if selected {
        0.08
    } else if response.hovered() {
        0.045
    } else {
        0.0
    };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if fill > 0.0 {
        painter.rect_filled(rect, 7, theme::tint(p.fg, fill));
    }
    // Neptune Video: a tab reached with the keyboard shows where it is.
    if response.has_focus() {
        focus_ring(ui.painter(), rect, 7, p);
    }
    // What waits in a tab is counted on it.
    let badge = (item.waiting > 0).then(|| {
        painter.layout_no_wrap(
            item.waiting.to_string(),
            theme::medium(10.5),
            p.legible(p.attention),
        )
    });
    let badge_width = badge.as_ref().map_or(0.0, |badge| badge.size().x + 12.0);
    let ink = if selected { p.fg } else { p.secondary };
    let named = if fit.icons {
        let width = ICON + badge_width;
        let glyph = Rect::from_min_size(
            Pos2::new(rect.center().x - width * 0.5, rect.center().y - ICON * 0.5),
            vec2(ICON, ICON),
        );
        icons::paint(&painter, glyph, item.icon, ink);
        glyph
    } else {
        let title = elided(
            &painter,
            item.label,
            theme::medium(fit.font),
            ink,
            (rect.width() - fit.padding - badge_width).max(0.0),
        );
        let width = title.size().x + badge_width;
        galley_at(
            &painter,
            Pos2::new(rect.center().x - width * 0.5, rect.center().y),
            title,
        )
    };
    if let Some(badge) = badge {
        let pill = Rect::from_min_max(
            Pos2::new(named.right() + 5.0, rect.center().y - 7.5),
            Pos2::new(
                named.right() + 5.0 + badge.size().x + 8.0,
                rect.center().y + 7.5,
            ),
        );
        painter.rect_filled(pill, 7.5, theme::tint(p.attention, 0.18));
        painter.galley(
            pill.center() - badge.size() * 0.5,
            badge,
            Color32::PLACEHOLDER,
        );
    }
    let chosen = response.clicked();
    if fit.icons {
        response.on_hover_text(item.label);
    }
    chosen
}

/// The fit the names of `tabs` take in a strip `width` wide: the roomiest
/// whose names all fit, or icons where none does.
pub fn fit(ui: &Ui, p: Palette, tabs: &[Tab], width: f32) -> Fit {
    widths(ui, p, tabs, width).0
}

/// The fit, and what each tab needs in it, with the room the gaps leave.
fn widths(ui: &Ui, p: Palette, tabs: &[Tab], width: f32) -> (Fit, Vec<f32>, f32) {
    let count = tabs.len().max(1);
    let mut fits = [Fit::ROOMY, Fit::TIGHT, Fit::ICONS].into_iter();
    let mut fit = fits.next().unwrap_or(Fit::ROOMY);
    loop {
        let room = width - fit.gap * (count - 1) as f32;
        let needed: Vec<f32> = tabs
            .iter()
            .map(|item| {
                let measure = |text: String, size| {
                    ui.painter()
                        .layout_no_wrap(text, theme::medium(size), p.fg)
                        .size()
                        .x
                };
                let badge = match item.waiting {
                    0 => 0.0,
                    count => measure(count.to_string(), 10.5) + 12.0,
                };
                if fit.icons {
                    ICON + badge + fit.padding * 2.0
                } else {
                    measure(item.label.to_owned(), fit.font) + badge + fit.padding
                }
            })
            .collect();
        if needed.iter().sum::<f32>() <= room {
            return (fit, needed, room);
        }
        match fits.next() {
            Some(next) => fit = next,
            None => return (fit, needed, room),
        }
    }
}

/// A strip of tabs in `strip`, `selected` among them. Each is exposed as
/// `role`, selected or not. Returns the tab chosen this frame, if another.
pub fn tabs(
    ui: &mut Ui,
    strip: Rect,
    p: Palette,
    tabs: &[Tab],
    selected: usize,
    role: WidgetType,
) -> Option<usize> {
    // Tabs are of equal width while every name fits one. In a panel too
    // narrow for that, each takes what its name needs and a share of the
    // rest; in one too narrow for that as well, names are set closer, and
    // then they give way to icons.
    let (fit, needed, room) = widths(ui, p, tabs, strip.width());
    let count = tabs.len().max(1) as f32;
    let each = room / count;
    let total = needed.iter().sum::<f32>();
    let spare = (room - total) / count;
    let fitted = fit.icons || needed.iter().all(|needed| *needed <= each);
    let mut left = strip.left();
    let mut chosen = None;
    for (index, (item, needed)) in tabs.iter().zip(needed).enumerate() {
        let width = if fitted {
            each
        } else if spare >= 0.0 {
            needed + spare
        } else {
            // Past every allowance each name gives up its share.
            needed * room / total
        };
        let cell = Rect::from_min_max(
            Pos2::new(left, strip.top() + 3.0),
            Pos2::new(left + width, strip.bottom() - 3.0),
        );
        left += width + fit.gap;
        if tab(ui, cell, p, *item, index == selected, role, fit) && index != selected {
            chosen = Some(index);
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, Appearance};
    use eframe::egui::{Event, Modifiers, PointerButton};

    const NAMES: [&str; 3] = ["Media", "Text", "Captions"];

    fn strip(width: f32, events: Vec<Event>) -> (Option<usize>, Fit, Vec<Rect>) {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let p = Palette::new(Appearance::Dark, Accent::default());
        theme::apply(&ctx, &p);
        let items = NAMES.map(|label| Tab {
            label,
            icon: Icon::Film,
            waiting: 0,
        });
        let mut result = (None, Fit::ROOMY, Vec::new());
        for events in [Vec::new(), events] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 200.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let rect = Rect::from_min_size(Pos2::new(10.0, 10.0), vec2(width, TABS));
                    let chosen = tabs(ui, rect, p, &items, 0, WidgetType::RadioButton);
                    let fit = fit(ui, p, &items, width);
                    result = (chosen, fit, Vec::new());
                },
            );
            output.textures_delta.clear();
        }
        result
    }

    #[test]
    fn names_share_a_roomy_strip_and_give_way_to_icons_in_a_narrow_one() {
        assert_eq!(strip(300.0, Vec::new()).1, Fit::ROOMY);
        assert_eq!(strip(160.0, Vec::new()).1, Fit::TIGHT);
        assert_eq!(strip(90.0, Vec::new()).1, Fit::ICONS);
    }

    #[test]
    fn a_click_chooses_another_tab() {
        // Three equal tabs of 100 points: the third is at 210 to 310.
        let at = Pos2::new(260.0, 27.0);
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let (chosen, ..) = strip(
            300.0,
            vec![Event::PointerMoved(at), press(true), press(false)],
        );
        assert_eq!(chosen, Some(2));
    }
}
