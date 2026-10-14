//! The toolbar's Export button, render queue button and project menu rows,
//! driven as a person drives them, and the room the project's name is left
//! at the narrowest and a wide window.
//!
//! The bench draws the toolbar in real egui passes over a window of a given
//! size, laid out as the window lays it out (after the window controls),
//! finds each control by the name accessibility is given for it and presses
//! it with the pointer.
use super::*;
use crate::theme::{Accent, Appearance};
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, Shape, accesskit::Role};

/// The window's narrowest and a common wide size.
const NARROWEST: Vec2 = vec2(720.0, 520.0);
const WIDE: Vec2 = vec2(1440.0, 900.0);
/// Where the window's own controls end and the toolbar begins.
const LEADING: f32 = 70.0;
const NAME: &str = "The documentary about the long walk home, second edit";

struct Seen {
    name: String,
    rect: Rect,
    role: Role,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    size: Vec2,
    time: f64,
    renders: QueueSummary,
    /// The save state: (dirty, saving, conflict).
    state: (bool, bool, bool),
    seen: Vec<Seen>,
    /// Text painted this frame, with where it was painted.
    painted: Vec<(String, Rect)>,
    actions: Vec<Action>,
}

impl Bench {
    fn new(size: Vec2) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut bench = Self {
            ctx,
            palette,
            size,
            time: 0.0,
            renders: QueueSummary::default(),
            state: (true, false, false),
            seen: Vec::new(),
            painted: Vec::new(),
            actions: Vec::new(),
        };
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert!(
            bench.actions.is_empty(),
            "FIXTURE: an untouched toolbar asked for {:?}",
            bench.actions
        );
        bench
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let project = editor_core::Project::new("Bench");
        let recent = Vec::new();
        let input = ToolbarInput {
            p: &self.palette,
            name: NAME,
            dirty: self.state.0,
            saved_once: true,
            saving: self.state.1,
            conflict: self.state.2,
            undo: Some("Move clip"),
            redo: Some("Trim clip"),
            recent: &recent,
            library: true,
            inspector: true,
            project: &project,
            clips: &[],
            playhead: Time::ZERO,
            freeze_hold: Time::from_seconds(2.0),
            renders: self.renders,
        };
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            let full = ui.max_rect();
            let bar = Rect::from_min_size(full.min, vec2(full.width(), metrics::TOOLBAR_HEIGHT));
            show(
                ui,
                bar.with_min_x(full.left() + LEADING),
                &input,
                &mut actions,
            );
        });
        if let Some(update) = &output.platform_output.accesskit_update {
            self.seen = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        rect: Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                    })
                })
                .collect();
        }
        self.painted.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.painted);
        }
        output.drop_without_applying_deltas();
        self.actions.extend(actions);
    }

    /// The control named `name`; in the menu when `in_menu`, otherwise on
    /// the bar.
    fn get(&self, name: &str, in_menu: bool) -> &Seen {
        (self.seen.iter())
            .find(|seen| {
                seen.name == name
                    && !matches!(seen.role, Role::Label | Role::TextRun)
                    && (seen.rect.top() >= metrics::TOOLBAR_HEIGHT) == in_menu
            })
            .unwrap_or_else(|| {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
            })
    }

    fn click(&mut self, name: &str, in_menu: bool) {
        let at = self.get(name, in_menu).rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(Vec::new());
    }

    /// Where the project's name was painted, however it was cut.
    fn name_rect(&self) -> Rect {
        let start: String = NAME.chars().take(6).collect();
        (self.painted.iter())
            .find(|(text, _)| text.starts_with(&start))
            .map(|(_, rect)| *rect)
            .expect("FIXTURE: the name is painted")
    }

    fn take(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
}

fn collect(shape: &Shape, into: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, into);
            }
        }
        Shape::Text(text) => {
            into.push((text.galley.text().to_owned(), shape.visual_bounding_rect()))
        }
        _ => {}
    }
}

#[test]
fn export_is_the_rightmost_control_with_the_queue_beside_it() {
    for size in [NARROWEST, WIDE] {
        let bench = Bench::new(size);
        let export = bench.get("Export", false).rect;
        let queue = bench.get("Render queue", false).rect;
        let settings = bench
            .get(&format!("Settings ({})", shortcut("Cmd+,")), false)
            .rect;
        assert!(
            export.right() <= size.x,
            "{size:?}: Export is inside the window"
        );
        assert!(
            (bench.seen.iter())
                .filter(|seen| seen.rect.top() < metrics::TOOLBAR_HEIGHT)
                .all(|seen| seen.rect.right() <= export.right() + 0.5),
            "{size:?}: nothing on the bar is right of Export"
        );
        assert!(
            queue.right() <= export.left(),
            "{size:?}: the queue is beside Export"
        );
        assert!(settings.right() <= queue.left(), "{size:?}: then Settings");
    }
}

#[test]
fn the_name_never_reaches_the_trailing_controls() {
    for size in [NARROWEST, WIDE] {
        let mut bench = Bench::new(size);
        for renders in [
            QueueSummary::default(),
            QueueSummary {
                active: 1,
                queued: 3,
                held: 2,
                progress: Some(0.4),
                rendering: Some(uuid::Uuid::new_v4()),
                paused: false,
            },
        ] {
            bench.renders = renders;
            bench.frame(Vec::new());
            let name = bench.name_rect();
            let leftmost = (bench.seen.iter())
                .filter(|seen| {
                    seen.rect.top() < metrics::TOOLBAR_HEIGHT
                        && seen.rect.left() > LEADING + 60.0
                        && !matches!(seen.role, Role::Label | Role::TextRun)
                })
                .map(|seen| seen.rect.left())
                .fold(f32::INFINITY, f32::min);
            let state = (bench.painted.iter())
                .find(|(text, _)| text == "Edited")
                .map(|(_, rect)| *rect);
            let right = state.map_or(name.right(), |state| state.right());
            assert!(
                right + 4.0 <= leftmost,
                "{size:?}: the name (and its state) end at {right}, the controls begin at {leftmost}"
            );
            assert!(name.width() >= 60.0, "{size:?}: the name keeps some room");
            assert!(
                state.is_some(),
                "{size:?}: the window says the project was edited"
            );
            if size == WIDE {
                assert!(name.width() > 300.0, "a wide window shows more of the name");
            }
        }
    }
}

#[test]
fn the_buttons_open_the_export_sheet_and_the_queue() {
    let mut bench = Bench::new(NARROWEST);
    bench.click("Export", false);
    assert!(matches!(bench.take().as_slice(), [Action::OpenExport]));
    bench.click("Render queue", false);
    assert!(matches!(bench.take().as_slice(), [Action::OpenRenderQueue]));
}

#[test]
fn the_project_menu_offers_export_and_the_queue_after_checkpoints() {
    let mut bench = Bench::new(WIDE);
    bench.click("Project menu", false);
    let checkpoints = bench.get("Checkpoints…", true).rect;
    let export = bench.get("Export…", true).rect;
    let queue = bench.get("Render queue", true).rect;
    assert!(checkpoints.top() < export.top() && export.top() < queue.top());
    bench.click("Export…", true);
    assert!(matches!(bench.take().as_slice(), [Action::OpenExport]));
    bench.click("Project menu", false);
    bench.click("Render queue", true);
    assert!(matches!(bench.take().as_slice(), [Action::OpenRenderQueue]));
}

#[test]
fn the_queue_button_says_how_far_the_queue_has_got() {
    let idle = QueueSummary::default();
    assert_eq!(
        queue_state(&idle),
        format!("Render queue ({})", shortcut("Shift+Cmd+E"))
    );
    let busy = QueueSummary {
        active: 1,
        queued: 2,
        held: 0,
        progress: Some(0.345),
        rendering: None,
        paused: false,
    };
    assert!(queue_state(&busy).starts_with("Render queue: rendering, 34 % · 2 waiting ("));
    // Jobs held by Add to queue are not called waiting: they wait for the
    // person, not for their turn.
    let held = QueueSummary { held: 2, ..busy };
    assert!(queue_state(&held).starts_with("Render queue: rendering, 34 % · 2 waiting · 2 held ("));
    let only_held = QueueSummary {
        held: 1,
        ..QueueSummary::default()
    };
    assert!(queue_state(&only_held).starts_with("Render queue: 1 held ("));
    let paused = QueueSummary {
        paused: true,
        queued: 0,
        ..busy
    };
    assert!(queue_state(&paused).starts_with("Render queue: paused at 34 %"));
}

#[test]
fn the_save_state_stays_beside_the_name_down_to_the_narrowest_window() {
    // The trailing controls are measured, not reserved: the state is shown
    // in words at every window from the narrowest up, and the name takes the
    // rest of the room before the controls.
    for width in [720.0, 760.0, 1440.0] {
        let size = vec2(width, 520.0);
        let mut bench = Bench::new(size);
        for (state, word) in [
            ((true, false, false), "Edited"),
            ((true, true, false), "Saving…"),
            ((true, false, true), "Changed on disk"),
        ] {
            bench.state = state;
            bench.frame(Vec::new());
            bench.frame(Vec::new());
            let painted = (bench.painted.iter())
                .find(|(text, _)| text == word)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| panic!("{width}: \"{word}\" is shown"));
            let leftmost = (bench.seen.iter())
                .filter(|seen| {
                    seen.rect.top() < metrics::TOOLBAR_HEIGHT
                        && seen.rect.left() > LEADING + 60.0
                        && !matches!(seen.role, Role::Label | Role::TextRun)
                })
                .map(|seen| seen.rect.left())
                .fold(f32::INFINITY, f32::min);
            assert!(
                painted.right() + 4.0 <= leftmost,
                "{width}: \"{word}\" ends at {} before the controls at {leftmost}",
                painted.right()
            );
            let name = bench.name_rect();
            // The name keeps what the state leaves of the free room.
            assert!(
                leftmost - painted.right() < 40.0 || name.width() >= 300.0,
                "{width}: the name {name:?} leaves room unused before {leftmost}"
            );
        }
    }
}

#[test]
fn where_the_words_do_not_fit_the_state_is_a_dot_with_its_name() {
    let mut bench = Bench::new(vec2(560.0, 520.0));
    bench.state = (true, false, true);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(
        !bench
            .painted
            .iter()
            .any(|(text, _)| text == "Changed on disk"),
        "no room for the words"
    );
    let dot = (bench.seen.iter())
        .find(|seen| seen.name == "Changed on disk")
        .expect("the conflict is still there to find");
    let name = bench.name_rect();
    assert!(
        dot.rect.left() >= name.right(),
        "{:?} after {name:?}",
        dot.rect
    );
    assert!(dot.rect.width() <= 8.0);
}
