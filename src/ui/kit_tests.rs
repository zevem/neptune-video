//! The kit's controls in real egui passes: how a select and a menu row light
//! under the pointer and with the keyboard (spec §3.10 and §9.12), and how
//! an empty state sets its hint.
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
