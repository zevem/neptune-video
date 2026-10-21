//! Where toasts are shown: in the window's top trailing corner, and under an
//! open sheet rather than over it; how long they stay, and what makes them
//! go. Drawn in real egui passes, with the render queue as the sheet.
use super::*;
use crate::ui::render_queue;
use editor_core::export::{ExportSettings, RenderJobInfo, RenderState};
use eframe::egui::{Modifiers, PointerButton, Shape};
use uuid::Uuid;

const NARROW: Vec2 = vec2(760.0, 560.0);

fn done(name: &str) -> RenderJobInfo {
    RenderJobInfo {
        id: Uuid::new_v4(),
        name: name.to_owned(),
        destination: PathBuf::from("/home/someone/Exports").join(name),
        settings: ExportSettings::default(),
        project_id: Uuid::new_v4(),
        revision: 1,
        state: RenderState::Done,
        progress: 1.0,
        frames_done: 10,
        frames_total: 10,
        speed_fps: 0.0,
        elapsed_seconds: 1.0,
        eta_seconds: None,
        encoder: None,
        output_bytes: Some(1000),
        context_id: Uuid::new_v4(),
        message: None,
        attempts: 1,
    }
}

fn notice(text: &str) -> Notice {
    Notice {
        text: text.to_owned(),
        problem: false,
        until: 1e9,
    }
}

fn problem(text: &str) -> Notice {
    Notice {
        text: text.to_owned(),
        problem: true,
        until: f64::INFINITY,
    }
}

/// A window of `NARROW` that shows `notices` as the application does: the
/// clocks first, then the toasts, the render queue open over them when
/// `jobs` is given.
struct Window {
    ctx: egui::Context,
    palette: Palette,
    notices: Vec<Notice>,
    jobs: Option<Vec<RenderJobInfo>>,
    time: f64,
    /// The words painted in the last frame, with where.
    texts: Vec<(String, Rect)>,
    /// The names accessibility was told of in the last frame, with where.
    named: Vec<(String, Rect)>,
    actions: Vec<Action>,
}

impl Window {
    fn new(notices: Vec<Notice>, jobs: Option<Vec<RenderJobInfo>>) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut window = Self {
            ctx,
            palette,
            notices,
            jobs,
            time: 0.0,
            texts: Vec::new(),
            named: Vec::new(),
            actions: Vec::new(),
        };
        for _ in 0..4 {
            window.frame(0.05, Vec::new());
        }
        window
    }

    /// One frame, `after` seconds after the last.
    fn frame(&mut self, after: f64, events: Vec<Event>) {
        self.time += after;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, NARROW)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            expire(ui.ctx(), &mut self.notices);
            if let Some(jobs) = &self.jobs {
                render_queue::show(ui.ctx(), &self.palette, jobs, &[]);
            }
            let over_sheet = self.jobs.is_some();
            notices(
                ui.ctx(),
                &self.palette,
                &mut self.notices,
                over_sheet,
                &mut actions,
            );
        });
        self.actions.extend(actions);
        self.texts.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.texts);
        }
        if let Some(update) = &output.platform_output.accesskit_update {
            self.named = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    let name = node.label().or_else(|| node.value())?.to_owned();
                    let rect = Rect::from_min_max(
                        pos2(bounds.x0 as f32, bounds.y0 as f32),
                        pos2(bounds.x1 as f32, bounds.y1 as f32),
                    );
                    Some((name, rect))
                })
                .collect();
        }
        output.drop_without_applying_deltas();
    }

    /// Where each notice's words were painted in the last frame, if they were.
    fn placed(&self) -> Vec<(String, Rect)> {
        let words = |notice: &Notice| {
            (self.texts.iter())
                .find(|(text, _)| *text == notice.text)
                .cloned()
        };
        self.notices.iter().filter_map(words).collect()
    }

    fn sheet(&self) -> Option<Rect> {
        (self.ctx).memory(|memory| memory.area_rect(Id::new("render-queue-sheet")))
    }

    fn named(&self, name: &str) -> Vec<Rect> {
        (self.named.iter())
            .filter(|(named, _)| named == name)
            .map(|(_, rect)| *rect)
            .collect()
    }

    fn click(&mut self, pos: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(0.05, vec![Event::PointerMoved(pos)]);
        self.frame(0.05, vec![button(true)]);
        self.frame(0.05, vec![button(false)]);
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
            into.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            ));
        }
        _ => {}
    }
}

#[test]
fn toasts_sit_at_the_window_s_top_trailing_corner() {
    let window = Window::new(
        vec![notice("Exported a.mp4"), notice("Exported b.mp4")],
        None,
    );
    let placed = window.placed();
    assert_eq!(placed.len(), 2, "both are shown: {placed:?}");
    let toolbar = metrics::TOOLBAR_HEIGHT;
    for (text, rect) in &placed {
        assert!(
            rect.right() <= NARROW.x - 14.0 && rect.right() > NARROW.x - 14.0 - 60.0,
            "{text} at the trailing edge: {rect:?}"
        );
        assert!(rect.top() > toolbar + 8.0, "{text} under the toolbar");
    }
    assert!(
        placed[0].1.top() < toolbar + 8.0 + 30.0,
        "the first at the top: {placed:?}"
    );
    assert!(placed[0].1.bottom() < placed[1].1.top(), "the newest last");
}

#[test]
fn notices_go_under_an_open_sheet_rather_than_over_it() {
    // The queue as tall as a 760 x 560 window lets it be.
    let jobs: Vec<RenderJobInfo> = (0..8).map(|at| done(&format!("job-{at}.mp4"))).collect();
    let window = Window::new(
        vec![
            notice("Exported job-6.mp4"),
            notice("Exported job-7.mp4"),
            notice("The file manager could not be opened"),
        ],
        Some(jobs),
    );
    let placed = window.placed();
    let sheet = window.sheet().expect("FIXTURE: the queue is open");
    assert!(
        sheet.top() < metrics::TOOLBAR_HEIGHT + 8.0 + 40.0,
        "FIXTURE: without the fix a toast at the window's top is over the sheet: {sheet:?}"
    );
    assert!(
        placed
            .iter()
            .any(|(text, _)| text == "The file manager could not be opened"),
        "the newest is shown: {placed:?}"
    );
    for (text, rect) in &placed {
        assert!(
            !rect.intersects(sheet),
            "{text} at {rect:?} is not over the sheet at {sheet:?}"
        );
        assert!(rect.bottom() <= NARROW.y, "{text} is inside the window");
        assert!(
            rect.right() <= NARROW.x - 14.0,
            "{text} at the trailing edge"
        );
    }
}

#[test]
fn a_problem_stays_until_it_is_dismissed() {
    let mut window = Window::new(
        vec![
            notice("Imported 1 file"),
            problem("The folder could not be read"),
        ],
        None,
    );
    // Information goes on its clock; a problem stays.
    window.notices[0].until = window.time + 4.0;
    window.frame(60.0, Vec::new());
    window.frame(0.05, Vec::new());
    let left: Vec<_> = window
        .notices
        .iter()
        .map(|notice| notice.text.clone())
        .collect();
    assert_eq!(left, ["The folder could not be read"]);
    assert_eq!(window.placed().len(), 1);

    // The toast is not a node; its Dismiss is.
    assert!(window.named("The folder could not be read").is_empty());
    let dismiss = window.named("Dismiss");
    assert_eq!(dismiss.len(), 1, "{:?}", window.named);
    let id = window.notices[0].id();
    window.click(dismiss[0].center());
    assert!(
        matches!(window.actions.as_slice(), [Action::DismissNotice(said)] if *said == id),
        "{:?}",
        window.actions
    );
}

#[test]
fn a_toast_under_the_pointer_keeps_the_time_it_had_left() {
    let mut window = Window::new(vec![notice("Copied 2 clips")], None);
    window.notices[0].until = window.time + 1.0;
    window.frame(0.05, Vec::new());
    let (_, words) = window.placed()[0].clone();
    // The pointer comes to rest on it, for longer than it had.
    window.frame(0.05, vec![Event::PointerMoved(words.center())]);
    window.frame(10.0, Vec::new());
    window.frame(0.05, Vec::new());
    assert_eq!(window.notices.len(), 1, "it waits while it is held");
    assert_eq!(window.placed().len(), 1);
    // Let go, it goes on from where it stopped, and then leaves.
    window.frame(0.05, vec![Event::PointerMoved(pos2(20.0, 500.0))]);
    window.frame(0.5, Vec::new());
    assert_eq!(window.notices.len(), 1, "not at once");
    window.frame(1.0, Vec::new());
    assert!(window.notices.is_empty());
}
