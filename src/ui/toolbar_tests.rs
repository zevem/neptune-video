//! The toolbar's launcher, Export button, render queue button, panel toggles
//! and project menu rows, driven as a person drives them, and the room the
//! project's name is left at the narrowest and a wide window.
//!
//! The bench draws the toolbar in real egui passes over a window of a given
//! size, with the side panels the window shows at that size, finds each
//! control by the name accessibility is given for it and presses it with the
//! pointer.
use super::*;
use crate::theme::{Accent, Appearance};
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, Shape, accesskit::Role};

/// The window's narrowest and a common wide size.
const NARROWEST: Vec2 = vec2(720.0, 520.0);
const WIDE: Vec2 = vec2(1440.0, 900.0);
const NAME: &str = "The documentary about the long walk home, second edit";

/// What the window shows beside the viewer at `width`, as `side_panels` and
/// `panel_widths` lay it out for a fresh window: both from 820, the
/// inspector alone below that, and the library alone where the inspector has
/// no room. The library's width, whether it is shown, and the viewer.
fn arrangement(width: f32) -> (f32, bool, Rangef) {
    let alone = |least: f32, most: f32| (width - 486.0).clamp(least, most);
    if width >= 820.0 {
        // Both panels shrink alike to leave the viewer 480.
        let short = ((316.0 + 300.0 + 480.0 - width) * 0.5).max(0.0);
        let (library, inspector) = ((316.0 - short).max(220.0), (300.0 - short).max(240.0));
        (library, true, Rangef::new(library, width - inspector))
    } else if width - 12.0 >= 560.0 {
        let inspector = alone(240.0, 300.0);
        (
            alone(220.0, 316.0),
            false,
            Rangef::new(6.0, width - inspector),
        )
    } else {
        let library = alone(220.0, 316.0);
        (library, true, Rangef::new(library, width - 6.0))
    }
}

/// A queue with work under way.
const BUSY: QueueSummary = QueueSummary {
    active: 1,
    queued: 1,
    held: 0,
    progress: Some(0.4),
    rendering: None,
    paused: false,
};

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
    name: &'static str,
    /// The save state: (dirty, saving, conflict).
    state: (bool, bool, bool),
    seen: Vec<Seen>,
    /// Text painted this frame, with where it was painted.
    painted: Vec<(String, Rect)>,
    /// The colours shapes other than text were painted in this frame.
    inks: Vec<egui::Color32>,
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
            name: NAME,
            state: (true, false, false),
            seen: Vec::new(),
            painted: Vec::new(),
            inks: Vec::new(),
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
        let (library_width, library, viewer) = arrangement(self.size.x);
        let input = ToolbarInput {
            p: &self.palette,
            name: self.name,
            dirty: self.state.0,
            saved_once: true,
            saving: self.state.1,
            conflict: self.state.2,
            undo: Some("Move clip"),
            redo: Some("Trim clip"),
            recent: &recent,
            library,
            library_width,
            library_reveal: if library { 1.0 } else { 0.0 },
            inspector: self.size.x >= 572.0,
            viewer,
            agent_waiting: false,
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
            show(ui, bar, &input, &mut actions);
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
        self.inks.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.painted);
            inks(&clipped.shape, &mut self.inks);
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
                    && (seen.rect.bottom() > metrics::TOOLBAR_HEIGHT) == in_menu
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

    /// The controls on the bar after the title, which the name and its
    /// state must stop short of.
    fn leftmost_after_title(&self) -> f32 {
        (self.seen.iter())
            .filter(|seen| {
                seen.rect.top() < metrics::TOOLBAR_HEIGHT
                    && !matches!(seen.role, Role::Label | Role::TextRun)
                    && !matches!(seen.name.as_str(), "Project menu" | "Media library")
            })
            .map(|seen| seen.rect.left())
            .fold(f32::INFINITY, f32::min)
    }

    /// Where the project's name was painted, however it was cut.
    fn name_rect(&self) -> Rect {
        let start: String = self.name.chars().take(5).collect();
        (self.painted.iter())
            .find(|(text, _)| text.starts_with(&start))
            .map(|(_, rect)| *rect)
            .expect("FIXTURE: the name is painted")
    }

    fn take(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
}

fn inks(shape: &Shape, into: &mut Vec<egui::Color32>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                inks(shape, into);
            }
        }
        Shape::Rect(rect) => into.extend([rect.fill, rect.stroke.color]),
        Shape::Circle(circle) => into.extend([circle.fill, circle.stroke.color]),
        Shape::Path(path) => {
            into.push(path.fill);
            if let egui::epaint::ColorMode::Solid(color) = path.stroke.color {
                into.push(color);
            }
        }
        Shape::LineSegment { stroke, .. } => into.push(stroke.color),
        Shape::Mesh(mesh) => into.extend(mesh.vertices.iter().map(|vertex| vertex.color)),
        _ => {}
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

// §5.1 and §7.5: the inspector's toggle is rightmost, Export before it and
// the queue beside Export; the Settings gear is gone.
#[test]
fn the_inspector_toggle_is_rightmost_with_export_and_the_queue_before_it() {
    for size in [NARROWEST, WIDE] {
        let mut bench = Bench::new(size);
        bench.renders = BUSY;
        bench.frame(Vec::new());
        let toggle = bench.get("Inspector and agent", false);
        assert_eq!(
            toggle.role,
            Role::CheckBox,
            "{size:?}: the toggle says it is on"
        );
        let toggle = toggle.rect;
        let export = bench.get("Export", false).rect;
        let queue = bench.get("Render queue", false).rect;
        assert!(
            toggle.right() <= size.x - 8.0 + 0.5,
            "{size:?}: the toggle is inside the window"
        );
        assert!(
            (bench.seen.iter())
                .filter(|seen| seen.rect.top() < metrics::TOOLBAR_HEIGHT)
                .all(|seen| seen.rect.right() <= toggle.right() + 0.5),
            "{size:?}: nothing on the bar is right of the inspector's toggle"
        );
        assert!(
            export.right() + 8.0 <= toggle.left() + 0.5,
            "{size:?}: Export is before it"
        );
        assert!(
            queue.right() + 4.0 <= export.left() + 0.5,
            "{size:?}: the queue is beside Export"
        );
        assert!(
            !(bench.seen.iter()).any(|seen| seen.name.starts_with("Settings")),
            "{size:?}: Preferences are in the project menu, not on the bar"
        );
    }
}

// §5.1: the queue's ring is on the bar only while the queue has jobs.
#[test]
fn the_queue_is_on_the_bar_only_while_it_has_jobs() {
    let mut bench = Bench::new(WIDE);
    assert!(!(bench.seen.iter()).any(|seen| seen.name == "Render queue"));
    bench.renders = QueueSummary {
        held: 1,
        ..QueueSummary::default()
    };
    bench.frame(Vec::new());
    bench.get("Render queue", false);
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
            let leftmost = bench.leftmost_after_title();
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
    // §5.1: the queue's ring is there while the queue has jobs.
    bench.renders = BUSY;
    bench.frame(Vec::new());
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
    // §5.7.2: the file's rows, then export, import, Preferences and the
    // clip's rows, each group after the last.
    let order = [
        "New project",
        "Open…",
        "Open recent",
        "Save",
        "Save as…",
        "Duplicate…",
        "Checkpoints…",
        "Export…",
        "Render queue",
        "Import media…",
        "Import folder…",
        "Preferences…",
        "Clip",
    ]
    .map(|name| bench.get(name, true).rect.top());
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the rows are in order: {order:?}"
    );
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
            let leftmost = bench.leftmost_after_title();
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

// §5.7.2: Preferences open from the project menu, as the gear did.
#[test]
fn the_project_menu_opens_preferences() {
    let mut bench = Bench::new(WIDE);
    bench.click("Project menu", false);
    bench.click("Preferences…", true);
    assert!(matches!(bench.take().as_slice(), [Action::OpenSettings]));
}

// §5.7.2: the clip's rows are a submenu of the project menu.
#[test]
fn the_clip_rows_open_from_their_submenu() {
    let mut bench = Bench::new(WIDE);
    bench.click("Project menu", false);
    let clip = bench.get("Clip", true).rect.center();
    for _ in 0..4 {
        bench.frame(vec![Event::PointerMoved(clip)]);
    }
    bench.click("Graph editor", true);
    assert!(matches!(bench.take().as_slice(), [Action::ToggleGraph]));
}

// §5.1: the launcher opens the agent's command line, as Ctrl+K does, whether
// it is a field over the viewer or folded to its glyph.
#[test]
fn the_launcher_opens_the_command_line() {
    let launcher = format!("Ask or type a command ({})", shortcut("Cmd+K"));
    for (size, field) in [(WIDE, true), (NARROWEST, false)] {
        let mut bench = Bench::new(size);
        bench.name = "Bench";
        bench.frame(Vec::new());
        let rect = bench.get(&launcher, false).rect;
        assert_eq!(rect.width() > 200.0, field, "{size:?}: {rect:?}");
        bench.click(&launcher, false);
        assert!(
            matches!(bench.take().as_slice(), [Action::OpenAgent { ask: false }]),
            "{size:?}"
        );
    }
}

// §5.1: the title button is the project menu; the name it shows is said too.
#[test]
fn the_title_says_the_name_of_the_project() {
    let bench = Bench::new(WIDE);
    let menu = bench.get("Project menu", false).rect;
    let name = (bench.seen.iter())
        .find(|seen| seen.name == NAME && seen.role == Role::Label)
        .expect("the name is said");
    assert!(menu.contains_rect(name.rect), "{:?} in {menu:?}", name.rect);
    // The library's toggle is before it, and says whether the library shows.
    let library = bench.get("Media library", false);
    assert_eq!(library.role, Role::CheckBox);
    assert!(library.rect.right() < menu.left());
}

// §5.1: the library's toggle and the inspector's answer the pointer.
#[test]
fn the_toggles_show_and_hide_the_panels() {
    let mut bench = Bench::new(WIDE);
    bench.click("Media library", false);
    assert!(matches!(bench.take().as_slice(), [Action::ToggleLibrary]));
    bench.click("Inspector and agent", false);
    assert!(matches!(bench.take().as_slice(), [Action::ToggleInspector]));
}

// §3.10: at rest the bar spends none of the accent: Export is quiet, and
// nothing is selected or focused.
#[test]
fn nothing_on_the_bar_is_accent_at_rest() {
    for size in [NARROWEST, WIDE] {
        let bench = Bench::new(size);
        let accent = bench.palette.accent;
        assert!(
            !bench.inks.contains(&accent),
            "{size:?}: the accent is painted at rest"
        );
    }
}
