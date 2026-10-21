//! Where toasts are shown: in the window's top trailing corner, and under an
//! open sheet rather than over it; how long they stay, and what makes them
//! go. Drawn in real egui passes, with the render queue as the sheet.
use super::*;
use crate::ui::render_queue;
use editor_core::export::{ExportSettings, RenderJobInfo, RenderState};
use eframe::egui::{Event, Modifiers, PointerButton, Shape};
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
        kind: NoticeKind::Info,
        until: 1e9,
    }
}

fn problem(text: &str) -> Notice {
    Notice {
        text: text.to_owned(),
        kind: NoticeKind::Problem,
        until: f64::INFINITY,
    }
}

/// A window of `NARROW`, or of `size`, that shows `notices` as the
/// application does: the clocks first, then the toasts, the render queue
/// open over them when `jobs` is given, and kept clear of the `cards` of
/// work over the viewer.
/// A button may stand where a toast can come, as the right panel's tabs do.
struct Window {
    ctx: egui::Context,
    size: Vec2,
    palette: Palette,
    notices: Vec<Notice>,
    jobs: Option<Vec<RenderJobInfo>>,
    cards: Vec<Rect>,
    beneath: Option<Rect>,
    time: f64,
    /// The words painted in the last frame, with where.
    texts: Vec<(String, Rect)>,
    /// What each text painted in the last frame shows, row by row, and
    /// whether its words were cut short.
    shown: Vec<(Vec<String>, bool)>,
    /// The names accessibility was told of in the last frame, with where.
    named: Vec<(String, Rect)>,
    actions: Vec<Action>,
    /// What the render queue answered.
    answers: Vec<render_queue::QueueAnswer>,
    /// How many times the button under the toasts was clicked.
    clicked_beneath: usize,
    /// How soon the last frame asked to be drawn again.
    wait: std::time::Duration,
}

impl Window {
    fn new(notices: Vec<Notice>, jobs: Option<Vec<RenderJobInfo>>) -> Self {
        Self::sized(NARROW, notices, jobs)
    }

    fn sized(size: Vec2, notices: Vec<Notice>, jobs: Option<Vec<RenderJobInfo>>) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut window = Self {
            ctx,
            size,
            palette,
            notices,
            jobs,
            cards: Vec::new(),
            beneath: None,
            time: 0.0,
            texts: Vec::new(),
            shown: Vec::new(),
            named: Vec::new(),
            actions: Vec::new(),
            answers: Vec::new(),
            clicked_beneath: 0,
            wait: std::time::Duration::MAX,
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
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            if let Some(beneath) = self.beneath
                && ui.put(beneath, egui::Button::new("Agent")).clicked()
            {
                self.clicked_beneath += 1;
            }
            // As the application does: drawn again when the next clock is up.
            let now = ui.input(|input| input.time);
            if let Some(next) = expire(ui.ctx(), &mut self.notices) {
                let wait = std::time::Duration::from_secs_f64((next - now).max(0.0));
                ui.ctx().request_repaint_after(wait);
            }
            if let Some(jobs) = &self.jobs
                && let Some(answer) = render_queue::show(ui.ctx(), &self.palette, jobs, &[], None)
            {
                self.answers.push(answer);
            }
            let over_sheet = self.jobs.is_some();
            notices(
                ui.ctx(),
                &self.palette,
                &mut self.notices,
                &self.cards,
                over_sheet,
                &mut actions,
            );
        });
        self.actions.extend(actions);
        self.wait = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        self.texts.clear();
        self.shown.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.texts);
            shown(&clipped.shape, &mut self.shown);
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

    /// Where each notice's toast was in the last frame, if it was shown:
    /// its whole surface, which reaches past its words on every side.
    fn placed(&self) -> Vec<(String, Rect)> {
        (self.notices.iter())
            .filter_map(|notice| {
                let rect = toast_rect(&self.ctx, notice)?;
                // FIXTURE: a toast in sight has its words painted.
                assert!(
                    (self.texts.iter())
                        .any(|(text, words)| *text == notice.text && rect.contains_rect(*words)),
                    "{} is painted on its toast",
                    notice.text
                );
                Some((notice.text.clone(), rect))
            })
            .collect()
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

/// What `shape` shows of its words, row by row, and whether they were cut.
fn shown(shape: &Shape, into: &mut Vec<(Vec<String>, bool)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                shown(shape, into);
            }
        }
        Shape::Text(text) => {
            let rows = (text.galley.rows.iter()).map(|row| row.text()).collect();
            into.push((rows, text.galley.elided));
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
            rect.right() <= NARROW.x - 14.0 + 0.5 && rect.right() > NARROW.x - 14.0 - 60.0,
            "{text} at the trailing edge: {rect:?}"
        );
        assert!(rect.top() >= toolbar + 8.0, "{text} under the toolbar");
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
            rect.right() <= NARROW.x - 14.0 + 0.5,
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
fn each_kind_of_notice_has_its_mark_and_a_warning_stays_as_a_problem_does() {
    // §5.8: Terminal marks what it did with a green check.
    for appearance in [Appearance::Dark, Appearance::Light] {
        let p = Palette::new(appearance, Accent::default());
        assert_eq!(NoticeKind::Info.mark(&p), (Icon::Info, p.secondary));
        assert_eq!(NoticeKind::Done.mark(&p), (Icon::CircleCheck, p.green));
        let warning = NoticeKind::Warning.mark(&p);
        assert_eq!(warning, (Icon::Warning, p.legible(p.yellow)));
        assert_eq!(NoticeKind::Problem.mark(&p), (Icon::AlertCircle, p.red));
    }
    assert_eq!(NoticeKind::outcome(false), NoticeKind::Done);
    assert_eq!(NoticeKind::outcome(true), NoticeKind::Problem);

    let mut window = Window::new(
        vec![
            Notice {
                kind: NoticeKind::Done,
                ..notice("Checkpoint “Rough cut” saved")
            },
            Notice {
                kind: NoticeKind::Warning,
                until: f64::INFINITY,
                ..notice("Unsaved changes were restored")
            },
        ],
        None,
    );
    // What was done goes on its clock; a warning waits to be read.
    window.notices[0].until = window.time + 4.0;
    window.frame(60.0, Vec::new());
    window.frame(0.05, Vec::new());
    let left: Vec<_> = (window.notices.iter())
        .map(|notice| notice.text.clone())
        .collect();
    assert_eq!(left, ["Unsaved changes were restored"]);
    assert_eq!(window.named("Dismiss").len(), 1, "{:?}", window.named);

    // The same words told again are of the more pressing kind.
    let mut notices = vec![Notice {
        kind: NoticeKind::Done,
        until: 5.0,
        ..notice("Saved")
    }];
    tell(
        &mut notices,
        Notice {
            kind: NoticeKind::Warning,
            ..notice("Saved")
        },
    );
    assert_eq!(notices[0].kind, NoticeKind::Warning);
    assert!(notices[0].until.is_infinite());
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

/// A warning and a problem stay until they are dismissed, and a window that
/// shows them and is left alone stops drawing, whether the pointer is away,
/// on a toast's words or on its Dismiss (performance.md, spec §5.8).
#[test]
fn a_window_showing_toasts_that_stay_goes_idle() {
    let mut window = Window::new(
        vec![
            Notice {
                kind: NoticeKind::Warning,
                ..problem("Unsaved changes were restored")
            },
            problem("The folder could not be read"),
        ],
        None,
    );
    let idle = |window: &mut Window, events: Vec<Event>| {
        window.frame(0.05, events);
        for _ in 0..8 {
            window.frame(1.0, Vec::new());
        }
        assert_eq!(window.placed().len(), 2);
        assert!(
            window.wait > std::time::Duration::from_secs(3600),
            "{:?}",
            window.wait
        );
    };
    idle(&mut window, Vec::new());
    let (_, words) = window.placed()[1].clone();
    idle(&mut window, vec![Event::PointerMoved(words.center())]);
    let dismiss = window.named("Dismiss");
    assert_eq!(dismiss.len(), 2, "{:?}", window.named);
    idle(&mut window, vec![Event::PointerMoved(dismiss[1].center())]);
    idle(&mut window, vec![Event::PointerMoved(pos2(20.0, 500.0))]);
    assert_eq!(window.notices.len(), 2, "they stay");
    // FIXTURE: news on its clock is drawn again when its time is up.
    let until = window.time + 4.0;
    tell(
        &mut window.notices,
        Notice {
            until,
            ..notice("Copied 2 clips")
        },
    );
    window.frame(0.05, Vec::new());
    assert!(
        window.wait <= std::time::Duration::from_secs(4),
        "{:?}",
        window.wait
    );
}

#[test]
fn a_dismiss_in_sight_answers_over_a_sheet_that_opened_after_it() {
    // The queue is as tall whatever it lists, and at 560 a problem's toast
    // does not fit under it and waits (spec §5.8); 640 leaves it room.
    let mut window = Window::sized(
        vec2(760.0, 640.0),
        vec![problem("The folder could not be read")],
        None,
    );
    // The sheet opens over a problem already shown: egui puts a new modal
    // on top of every layer, and its backdrop would take the click.
    window.jobs = Some(vec![done("a.mp4")]);
    for _ in 0..4 {
        window.frame(0.05, Vec::new());
    }
    let sheet = window.sheet().expect("FIXTURE: the queue is open");
    let dismiss = window.named("Dismiss");
    assert_eq!(dismiss.len(), 1, "{:?}", window.named);
    assert!(!dismiss[0].intersects(sheet), "FIXTURE: beside the sheet");
    let id = window.notices[0].id();
    window.click(dismiss[0].center());
    assert!(
        matches!(window.actions.as_slice(), [Action::DismissNotice(said)] if *said == id),
        "{:?}",
        window.actions
    );
    assert!(
        window.answers.is_empty(),
        "the sheet heard nothing: {:?}",
        window.answers
    );
}

#[test]
fn a_toast_lets_the_pointer_through_to_what_is_under_it() {
    let mut window = Window::new(vec![notice("Saved")], None);
    window.frame(0.05, Vec::new());
    let (_, words) = window.placed()[0].clone();
    // A tab under the toast, as the right panel's are.
    window.beneath = Some(Rect::from_center_size(words.center(), vec2(120.0, 28.0)));
    window.frame(0.05, Vec::new());
    window.click(words.center());
    assert_eq!(window.clicked_beneath, 1, "the tab under it is clicked");
    assert!(window.actions.is_empty(), "{:?}", window.actions);
    // Resting on it still holds it.
    window.notices[0].until = window.time + 0.5;
    window.frame(0.05, Vec::new());
    window.frame(10.0, Vec::new());
    assert_eq!(window.notices.len(), 1, "it waits while it is held");
}

#[test]
fn toasts_keep_clear_of_the_cards_of_work() {
    let mut window = Window::new(
        vec![
            notice("Exported a file with a name long enough to reach the card.mp4"),
            problem("The folder could not be read"),
        ],
        None,
    );
    // A viewer that reaches the window's trailing edge, cards at its top.
    window.cards = vec![
        Rect::from_min_size(pos2(380.0, 50.0), vec2(360.0, 60.0)),
        Rect::from_min_size(pos2(380.0, 118.0), vec2(360.0, 60.0)),
    ];
    window.frame(0.05, Vec::new());
    window.frame(0.05, Vec::new());
    let placed = window.placed();
    assert_eq!(placed.len(), 2, "both are shown: {placed:?}");
    for (text, rect) in &placed {
        for card in &window.cards {
            assert!(
                !rect.intersects(*card),
                "{text} at {rect:?} is over {card:?}"
            );
        }
    }
    assert!(placed[0].1.bottom() < placed[1].1.top(), "the newest last");
    for dismiss in window.named("Dismiss") {
        for card in &window.cards {
            assert!(!dismiss.intersects(*card), "Dismiss is over a card");
        }
    }
}

fn told(notices: &[Notice]) -> Vec<(&str, bool)> {
    (notices.iter())
        .map(|notice| (notice.text.as_str(), notice.kind == NoticeKind::Problem))
        .collect()
}

#[test]
fn the_same_words_again_are_one_notice_and_a_problem_stays_one() {
    let mut notices = vec![problem("The folder could not be read")];
    // Said again as information, it is still a problem with no end.
    tell(
        &mut notices,
        Notice {
            until: 10.0,
            ..notice("The folder could not be read")
        },
    );
    assert_eq!(told(&notices), [("The folder could not be read", true)]);
    assert!(notices[0].until.is_infinite());
    // Information said again stays the longer of the two.
    let mut notices = vec![Notice {
        until: 20.0,
        ..notice("Saved")
    }];
    tell(
        &mut notices,
        Notice {
            until: 12.0,
            ..notice("Saved")
        },
    );
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].until, 20.0);
    // Information that turns out a problem becomes one.
    tell(&mut notices, problem("Saved"));
    assert_eq!(told(&notices), [("Saved", true)]);
    assert!(notices[0].until.is_infinite());
}

#[test]
fn past_four_information_goes_first_and_never_takes_a_problem_s_place() {
    let mut notices = vec![
        problem("One"),
        notice("Two"),
        problem("Three"),
        notice("Four"),
    ];
    tell(&mut notices, notice("Five"));
    assert_eq!(
        told(&notices),
        [
            ("One", true),
            ("Three", true),
            ("Four", false),
            ("Five", false)
        ]
    );
    // §5.8, docs/design.md: persistent failures remain discoverable, and
    // brief success feedback does not take one away. Among problems only,
    // information just told is not shown at all.
    let mut notices = vec![problem("A"), problem("B"), problem("C"), problem("D")];
    tell(&mut notices, notice("Saved"));
    assert_eq!(
        told(&notices),
        [("A", true), ("B", true), ("C", true), ("D", true)]
    );
    // A fifth problem takes the place of the one in sight the longest.
    tell(&mut notices, problem("E"));
    assert_eq!(
        told(&notices),
        [("B", true), ("C", true), ("D", true), ("E", true)]
    );
}

#[test]
fn toasts_keep_clear_of_a_sheet_a_question_was_asked_over() {
    // The export sheet, and a question asked over it: the question is the
    // top modal, and the sheet is still in sight under it.
    let ctx = egui::Context::default();
    let palette = Palette::new(Appearance::Dark, Accent::default());
    theme::install_fonts(&ctx);
    theme::apply(&ctx, &palette);
    let mut told = vec![notice("Exported a.mp4")];
    let mut time = 0.0;
    for _ in 0..5 {
        time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, NARROW)),
            time: Some(time),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            egui::Modal::new(Id::new("export-sheet"))
                .frame(egui::Frame::new().fill(palette.elevated))
                .show(ctx, |ui| {
                    ui.allocate_exact_size(vec2(NARROW.x - 24.0, 480.0), Sense::hover());
                });
            replace_export(ctx, &palette, "a.mp4", None, "Exports");
            notices(ctx, &palette, &mut told, &[], true, &mut Vec::new());
        });
        output.drop_without_applying_deltas();
    }
    let export = (ctx.memory(|memory| memory.area_rect(Id::new("export-sheet"))))
        .expect("FIXTURE: the export sheet is open");
    let asked = (ctx.memory(|memory| memory.area_rect(Id::new("replace-export-sheet"))))
        .expect("FIXTURE: the question is open");
    let toast = toast_rect(&ctx, &told[0]).expect("there is room under the sheet");
    assert!(
        !toast.intersects(asked),
        "FIXTURE: beside the question at {asked:?}"
    );
    assert!(
        !toast.intersects(export),
        "the toast at {toast:?} is not over the sheet at {export:?}"
    );
    assert!(toast.bottom() <= NARROW.y);
}

#[test]
fn a_problem_says_every_word_and_its_mark_and_dismiss_share_a_line() {
    // §5.8: a problem stays to be read, so it is never cut short; here the
    // words cut would be where the person's take is kept.
    let words = "The take could not be placed: the clip would overlap another \
                 on a locked track, and a locked track is never changed by a take. \
                 It is kept in \
                 /home/someone/Videos/Neptune Video/Takes/Interview take 3 \
                 2026-10-21 14-32-07.mkv";
    let mut window = Window::sized(vec2(1440.0, 900.0), vec![problem(words)], None);
    window.frame(0.05, Vec::new());
    let (_, toast) = window.placed()[0].clone();
    let (rows, cut) = (window.shown.iter())
        .find(|(rows, _)| rows.len() > 1 && rows.concat().contains("could not be placed"))
        .expect("FIXTURE: the words are painted on more than one row")
        .clone();
    assert!(!cut, "every word is painted: {rows:?}");
    assert!(rows.len() > 3, "FIXTURE: longer than three rows: {rows:?}");
    assert!(
        rows.last().is_some_and(|row| row.ends_with("14-32-07.mkv")),
        "{rows:?}"
    );
    // The mark and the Dismiss are both on the toast's centre line, as in
    // Terminal's notice.
    let dismiss = window.named("Dismiss");
    assert_eq!(dismiss.len(), 1, "{:?}", window.named);
    assert!((dismiss[0].center().y - toast.center().y).abs() < 0.5);
    let mark = (window.texts.iter())
        .filter(|(_, rect)| toast.contains_rect(*rect) && rect.right() < toast.left() + 40.0)
        .map(|(_, rect)| *rect)
        .next()
        .expect("FIXTURE: the mark is painted at the leading edge");
    assert!(
        (mark.center().y - toast.center().y).abs() < 2.0,
        "mark at {mark:?} on the toast's centre line {toast:?}"
    );

    // Information goes by itself and keeps to three rows.
    let window = Window::sized(vec2(1440.0, 900.0), vec![notice(words)], None);
    let (rows, cut) = (window.shown.iter())
        .find(|(rows, _)| rows.concat().contains("could not be placed"))
        .expect("FIXTURE: the words are painted")
        .clone();
    assert!(cut && rows.len() == 3, "{rows:?}");
}
