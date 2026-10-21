//! The kit's controls in real egui passes: how a select, a menu row, a chip
//! and a switch light under the pointer and with the keyboard (spec §3.10
//! and §9.12), how a disabled button writes its words, and how an empty
//! state sets its hint.
use super::*;
use crate::theme::{Accent, Appearance};
use eframe::egui::{Event, Modifiers, PointerButton, epaint::RectShape};
use std::time::Duration;

fn palette() -> Palette {
    Palette::new(Appearance::Dark, Accent::default())
}

/// A window of 480 × 320 that draws `add` in each frame: what it painted
/// last, and how soon it asked to be drawn again.
struct Bench {
    ctx: egui::Context,
    shapes: Vec<Shape>,
    wait: Duration,
}

impl Bench {
    fn new() -> Self {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette());
        Self {
            ctx,
            shapes: Vec::new(),
            wait: Duration::MAX,
        }
    }

    fn frame(&mut self, events: Vec<Event>, add: &mut dyn FnMut(&mut Ui)) {
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 320.0))),
                events,
                ..Default::default()
            },
            |ui| add(ui),
        );
        output.textures_delta.clear();
        self.wait = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        fn flatten(shape: Shape, into: &mut Vec<Shape>) {
            match shape {
                Shape::Vec(shapes) => shapes.into_iter().for_each(|shape| flatten(shape, into)),
                shape => into.push(shape),
            }
        }
        self.shapes.clear();
        for clipped in output.shapes {
            flatten(clipped.shape, &mut self.shapes);
        }
    }

    fn rects(&self) -> impl Iterator<Item = &RectShape> {
        self.shapes.iter().filter_map(|shape| match shape {
            Shape::Rect(rect) => Some(rect),
            _ => None,
        })
    }

    fn filled(&self, rect: Rect, fill: Color32) -> bool {
        self.rects()
            .any(|shape| shape.rect == rect && shape.fill == fill)
    }

    /// Whether the ring of `focus_ring` is drawn round `rect`.
    fn ringed(&self, rect: Rect) -> bool {
        let ring = theme::tint(palette().accent, 0.55);
        (self.rects()).any(|shape| shape.rect == rect.expand(2.0) && shape.stroke.color == ring)
    }

    /// The ink `text` was painted in.
    fn ink(&self, text: &str) -> Option<Color32> {
        self.shapes.iter().find_map(|shape| match shape {
            Shape::Text(shape) if shape.galley.text() == text => (shape.galley.job.sections)
                .first()
                .map(|section| section.format.color)
                .filter(|color| *color != Color32::PLACEHOLDER)
                .or(Some(shape.fallback_color)),
            _ => None,
        })
    }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn a_select_lies_on_pressed_under_the_pointer_and_rings_with_the_keyboard() {
    // Terminal's select: no fill at rest, `pressed` under the pointer and
    // while its list is open, the focus ring with the keyboard.
    let p = palette();
    let id = Id::new("select");
    let mut bench = Bench::new();
    let field = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        let size = vec2(200.0, metrics::CONTROL_HEIGHT);
        field.set(select(ui, &p, id, "H.264", "Codec", size).rect);
    };
    bench.frame(Vec::new(), &mut add);
    let rect = field.get();
    assert!(!bench.filled(rect, p.pressed), "no fill at rest");
    assert!(!bench.ringed(rect));
    bench.frame(vec![Event::PointerMoved(rect.center())], &mut add);
    assert!(bench.filled(rect, p.pressed), "under the pointer");
    bench.frame(vec![press(rect.center(), true)], &mut add);
    assert!(bench.filled(rect, p.pressed), "pressed");
    bench.frame(vec![Event::PointerGone], &mut add);
    let focused = controls::viewport_id(&bench.ctx, id);
    bench.ctx.memory_mut(|memory| memory.request_focus(focused));
    bench.frame(Vec::new(), &mut add);
    assert!(bench.ringed(rect), "with the keyboard");
    bench.frame(Vec::new(), &mut add);
    assert_eq!(bench.wait, Duration::MAX, "and it rests");
}

#[test]
fn a_menu_row_lies_on_the_accent_under_the_pointer_and_with_the_keyboard() {
    // Terminal's menu row: the row the pointer or the arrows are on is the
    // accent, its words `on_accent`; a destructive one is red.
    let p = palette();
    for destructive in [false, true] {
        let mut bench = Bench::new();
        let row = std::cell::Cell::new((Rect::NOTHING, Id::NULL));
        let mut add = |ui: &mut Ui| {
            ui.set_width(240.0);
            let response = menu_row(ui, &p, Some(Icon::Trash), "Delete", "", destructive, None);
            row.set((response.rect, response.id));
        };
        bench.frame(Vec::new(), &mut add);
        let (rect, id) = row.get();
        let (fill, on) = if destructive {
            (p.red, p.on_red())
        } else {
            (p.accent, p.on_accent)
        };
        assert!(!bench.filled(rect, fill), "no fill at rest");
        assert_ne!(bench.ink("Delete"), Some(on));
        bench.frame(vec![Event::PointerMoved(rect.center())], &mut add);
        assert!(bench.filled(rect, fill), "under the pointer");
        assert_eq!(bench.ink("Delete"), Some(on));
        bench.frame(vec![press(rect.center(), true)], &mut add);
        assert!(bench.filled(rect, fill), "pressed");
        bench.frame(
            vec![press(rect.center(), false), Event::PointerGone],
            &mut add,
        );
        bench.ctx.memory_mut(|memory| memory.request_focus(id));
        bench.frame(Vec::new(), &mut add);
        assert!(bench.filled(rect, fill), "with the keyboard");
        bench.frame(Vec::new(), &mut add);
        assert_eq!(bench.wait, Duration::MAX, "and it rests");
    }
}

#[test]
fn a_chip_lies_on_hover_and_pressed_and_the_chosen_one_is_checked() {
    // §3.10: `hover` eased in under the pointer, `pressed` while held and
    // when chosen, the check before a chosen one's name, and the ring with
    // the keyboard.
    let p = palette();
    for selected in [false, true] {
        let mut bench = Bench::new();
        let chip = std::cell::Cell::new((Rect::NOTHING, Id::NULL));
        let mut add = |ui: &mut Ui| {
            let response = super::chip(ui, &p, "Selected clips", selected);
            chip.set((response.rect, response.id));
        };
        bench.frame(Vec::new(), &mut add);
        let (rect, id) = chip.get();
        assert_eq!(bench.filled(rect, p.pressed), selected, "at rest");
        assert!(!bench.filled(rect, p.hover));
        assert!(!bench.ringed(rect));
        // The check is a glyph, which is painted as words are.
        let checked = (bench.shapes.iter()).any(|shape| match shape {
            Shape::Text(shape) => shape.galley.text() != "Selected clips",
            _ => false,
        });
        assert_eq!(checked, selected, "the check");
        bench.frame(vec![Event::PointerMoved(rect.center())], &mut add);
        for _ in 0..30 {
            bench.frame(Vec::new(), &mut add);
        }
        assert!(bench.filled(rect, p.hover), "under the pointer");
        bench.frame(vec![press(rect.center(), true)], &mut add);
        assert!(bench.filled(rect, p.pressed), "pressed");
        bench.frame(
            vec![press(rect.center(), false), Event::PointerGone],
            &mut add,
        );
        for _ in 0..30 {
            bench.frame(Vec::new(), &mut add);
        }
        assert!(!bench.filled(rect, p.hover), "the pointer has gone");
        bench.ctx.memory_mut(|memory| memory.request_focus(id));
        bench.frame(Vec::new(), &mut add);
        assert!(bench.ringed(rect), "with the keyboard");
        bench.frame(Vec::new(), &mut add);
        assert_eq!(bench.wait, Duration::MAX, "and it rests");
    }
}

#[test]
fn a_switch_rings_with_the_keyboard() {
    let p = palette();
    let mut bench = Bench::new();
    let switch = std::cell::Cell::new((Rect::NOTHING, Id::NULL));
    let mut on = false;
    let mut add = |ui: &mut Ui| {
        let response = controls::toggle(ui, p, &mut on, "Reduce motion");
        switch.set((response.rect, response.id));
    };
    bench.frame(Vec::new(), &mut add);
    let (rect, id) = switch.get();
    assert!(!bench.ringed(rect), "a ring at rest");
    bench.ctx.memory_mut(|memory| memory.request_focus(id));
    bench.frame(Vec::new(), &mut add);
    assert!(bench.ringed(rect), "with the keyboard");
}

#[test]
fn a_disabled_button_writes_its_words_muted_as_a_disabled_chip_does() {
    // F12: one disabled strength. Secondary and Quiet words are `muted`, as
    // a disabled chip's are; a solid button keeps its ink on its fill.
    let p = palette();
    for (kind, label, enabled) in [
        (ButtonKind::Secondary, "Clear proxies", p.fg),
        (ButtonKind::Quiet, "Show all", p.secondary),
        (ButtonKind::Primary, "Export", p.on_accent),
    ] {
        let mut bench = Bench::new();
        for disabled in [false, true] {
            bench.frame(Vec::new(), &mut |ui: &mut Ui| {
                if disabled {
                    ui.disable();
                }
                controls::button(ui, p, label, kind);
                super::chip(ui, &p, "Selected clips", false);
            });
            // The disabled painter halves whatever it is given.
            let halved = |ink: Color32| {
                ink.gamma_multiply(bench.ctx.global_style().visuals.disabled_alpha())
            };
            let muted = disabled && kind != ButtonKind::Primary;
            let ink = match (muted, disabled) {
                (true, _) => halved(p.muted),
                (false, true) => halved(enabled),
                (false, false) => enabled,
            };
            assert_eq!(bench.ink(label), Some(ink), "{label}, disabled {disabled}");
            if disabled {
                assert_eq!(bench.ink("Selected clips"), Some(halved(p.muted)));
            }
        }
    }
}

#[test]
fn an_empty_state_s_hint_leaves_no_word_alone_on_its_last_line() {
    // Spec §4.3.8: a centred, wrapped hint, its lines as even as the words
    // allow. The library's at its default width of 316 is two lines, and
    // the second is not one word.
    const HINT: &str = "Import video, sound and images, or drop them here.";
    let p = palette();
    let mut bench = Bench::new();
    let mut add = |ui: &mut Ui| {
        let rect = Rect::from_min_size(pos2(8.0, 0.0), vec2(302.0, 320.0));
        empty_state(ui, &p, rect, Icon::Film, "No media yet", HINT, None);
    };
    bench.frame(Vec::new(), &mut add);
    let hint = (bench.shapes.iter())
        .find_map(|shape| match shape {
            Shape::Text(shape) if shape.galley.text() == HINT => Some(shape.galley.clone()),
            _ => None,
        })
        .expect("the hint is painted");
    let rows: Vec<f32> = (hint.rows.iter()).map(|row| row.rect().width()).collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let alone = hint.rows[1].glyphs.iter().all(|glyph| glyph.chr != ' ');
    assert!(!alone, "the last line holds more than a word");
    assert!((rows[0] - rows[1]).abs() < 40.0, "even lines: {rows:?}");
    // At no more than its 260 points.
    assert!(rows.iter().all(|width| *width <= 260.0), "{rows:?}");
    // A hint that fits one line keeps it.
    let mut add = |ui: &mut Ui| {
        let rect = Rect::from_min_size(pos2(8.0, 0.0), vec2(302.0, 320.0));
        empty_state(
            ui,
            &p,
            rect,
            Icon::Search,
            "No matches",
            "Try other words.",
            None,
        );
    };
    bench.frame(Vec::new(), &mut add);
    let one = (bench.shapes.iter()).find_map(|shape| match shape {
        Shape::Text(shape) if shape.galley.text() == "Try other words." => {
            Some(shape.galley.rows.len())
        }
        _ => None,
    });
    assert_eq!(one, Some(1));
}

/// The words and glyphs of each text shape `shape` paints, with the opacity
/// each is struck at: a word struck twice is found twice.
fn strikes(shape: &Shape, found: &mut Vec<(String, f32)>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| strikes(shape, found)),
        Shape::Text(text) => found.push((text.galley.text().to_owned(), text.opacity_factor)),
        _ => {}
    }
}

/// What `add` strikes in one pass of a window in `appearance`.
fn struck_in(appearance: Appearance, add: impl FnMut(&mut Ui)) -> Vec<(String, f32)> {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, &Palette::new(appearance, Accent::Blue));
    let mut output = ctx.run_ui(egui::RawInput::default(), add);
    output.textures_delta.clear();
    let mut found = Vec::new();
    for clipped in &output.shapes {
        strikes(&clipped.shape, &mut found);
    }
    found
}

/// Text on a dark ground is struck twice where the glyph atlas holds plain
/// coverage, as in the light appearance, and once in the dark, whose atlas
/// already holds `2c - c²`. Translucent text keeps its core: two strikes at
/// `1 - √(1 - a)` cover `a`. Text painted before the dark ground, the
/// chrome's, is left as it is.
#[test]
fn text_on_a_dark_ground_keeps_its_dark_weight_in_either_appearance() {
    for (appearance, twice) in [(Appearance::Light, true), (Appearance::Dark, false)] {
        let found = struck_in(appearance, |ui| {
            let painter = ui.painter();
            let font = theme::mono(15.0);
            painter.text(
                pos2(0.0, 0.0),
                Align2::LEFT_TOP,
                "chrome",
                font.clone(),
                Color32::BLACK,
            );
            let from = first_shape(painter);
            painter.text(
                pos2(0.0, 20.0),
                Align2::LEFT_TOP,
                "opaque",
                font.clone(),
                Color32::WHITE,
            );
            let half = Color32::WHITE.gamma_multiply(0.5);
            painter.text(pos2(0.0, 40.0), Align2::LEFT_TOP, "half", font, half);
            keep_text_weight(painter, from);
        });
        let count = |name: &str| found.iter().filter(|(text, _)| text == name).count();
        assert_eq!(count("chrome"), 1, "{appearance:?}");
        assert_eq!(count("opaque"), if twice { 2 } else { 1 }, "{appearance:?}");
        assert_eq!(count("half"), if twice { 2 } else { 1 }, "{appearance:?}");
        let opacity = |name: &str| found.iter().find(|(text, _)| text == name).map(|(_, o)| *o);
        assert_eq!(opacity("opaque"), Some(1.0));
        let a = f32::from(Color32::WHITE.gamma_multiply(0.5).a()) / 255.0;
        let struck = if twice {
            (1.0 - (1.0 - a).sqrt()) / a
        } else {
            1.0
        };
        let half = opacity("half").expect("the translucent text is painted");
        assert!(
            (half - struck).abs() < 1e-4,
            "{appearance:?}: {half} against {struck}"
        );
        if twice {
            // Two strikes at that opacity reach the one strike's core.
            let core = 1.0 - (1.0 - a * half).powi(2);
            assert!((core - a).abs() < 1e-4, "{core} against {a}");
        }
    }
}

/// A badge over a picture says its words at their dark weight in either
/// appearance, and what `on_dark` paints a glyph in is struck the same way,
/// while the words beside it are not (spec §6).
#[test]
fn a_badge_and_a_glyph_on_a_dark_well_keep_their_dark_weight() {
    for (appearance, twice) in [(Appearance::Light, 2), (Appearance::Dark, 1)] {
        let found = struck_in(appearance, |ui| {
            let painter = ui.painter();
            badge(painter, pos2(10.0, 10.0), "0:08");
            on_dark(painter, || {
                icons::paint(
                    painter,
                    Rect::from_min_size(pos2(10.0, 40.0), Vec2::splat(14.0)),
                    Icon::Play,
                    Color32::WHITE,
                );
            });
            painter.text(
                pos2(10.0, 60.0),
                Align2::LEFT_TOP,
                "Name",
                theme::medium(13.0),
                Color32::BLACK,
            );
        });
        let count = |name: &str| found.iter().filter(|(text, _)| text == name).count();
        assert_eq!(count("0:08"), twice, "{appearance:?}");
        assert_eq!(found.len(), 2 * twice + 1, "{appearance:?}: {found:?}");
        assert_eq!(count("Name"), 1, "{appearance:?}");
    }
}
