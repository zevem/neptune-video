//! The render queue panel driven as a person drives it.
//!
//! [`Bench`] shows the panel in real egui passes over a window of a given
//! size with jobs written by hand, finds each control by the name
//! accessibility is given for it and presses it with the pointer.
//!
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the panel.
use super::*;
use crate::theme::{Accent, Appearance};
use editor_core::export::ExportSettings;
use eframe::egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Shape, accesskit::Role};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

const WIDE: Vec2 = vec2(1440.0, 900.0);
const NARROW: Vec2 = vec2(760.0, 560.0);

fn job(name: &str, state: RenderState) -> RenderJobInfo {
    RenderJobInfo {
        id: Uuid::new_v4(),
        name: name.to_owned(),
        destination: PathBuf::from("/home/someone/Exports").join(name),
        settings: ExportSettings::default(),
        project_id: Uuid::new_v4(),
        revision: 3,
        state,
        progress: 0.0,
        frames_done: 0,
        frames_total: 360,
        speed_fps: 0.0,
        elapsed_seconds: 0.0,
        eta_seconds: None,
        encoder: None,
        output_bytes: None,
        context_id: Uuid::new_v4(),
        message: None,
        attempts: 0,
    }
}

/// A queue with a job in every state, in the order a queue lists them.
fn every_state() -> Vec<RenderJobInfo> {
    let mut rendering = job("rendering.mp4", RenderState::Rendering);
    rendering.progress = 0.34;
    rendering.frames_done = 120;
    rendering.speed_fps = 42.0;
    rendering.elapsed_seconds = 3.0;
    rendering.eta_seconds = Some(62.0);
    rendering.encoder = Some("h264_nvenc".to_owned());
    let mut paused = job("paused.mp4", RenderState::Paused);
    paused.progress = 0.5;
    paused.frames_done = 180;
    paused.elapsed_seconds = 42.0;
    let mut done = job("done.mp4", RenderState::Done);
    done.progress = 1.0;
    done.frames_done = 360;
    done.elapsed_seconds = 62.0;
    done.output_bytes = Some(45_200_000);
    done.encoder = Some("libx264".to_owned());
    done.message = Some("NVENC failed, so it was rendered in software".to_owned());
    let mut failed = job("failed.mp4", RenderState::Failed);
    failed.elapsed_seconds = 12.0;
    failed.message = Some("FFmpeg stopped: No space left on device".to_owned());
    let mut cancelled = job("cancelled.mp4", RenderState::Cancelled);
    cancelled.frames_done = 90;
    vec![
        rendering,
        job("preparing.mp4", RenderState::Preparing),
        job("finalizing.mp4", RenderState::Finalizing),
        paused,
        job("first.mp4", RenderState::Queued),
        job("second.mp4", RenderState::Queued),
        done,
        failed,
        cancelled,
    ]
}

#[derive(Debug)]
struct Seen {
    name: String,
    rect: Rect,
    role: Role,
    disabled: bool,
    value: Option<f64>,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    size: Vec2,
    time: f64,
    jobs: Vec<RenderJobInfo>,
    /// Paused jobs that are held: waiting, never started.
    held: Vec<Uuid>,
    seen: Vec<Seen>,
    painted: Vec<(String, Color32)>,
    answers: Vec<QueueAnswer>,
}

impl Bench {
    fn new(size: Vec2, jobs: Vec<RenderJobInfo>) -> Self {
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
            jobs,
            held: Vec::new(),
            seen: Vec::new(),
            painted: Vec::new(),
            answers: Vec::new(),
        };
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        assert!(
            bench.answers.is_empty(),
            "FIXTURE: a panel nobody touched answered {:?}",
            bench.answers
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
        // The queue's waiting jobs, in the order listed, held ones included.
        let waiting: Vec<Uuid> = (self.jobs.iter())
            .filter(|job| job.state == RenderState::Queued || self.held.contains(&job.id))
            .map(|job| job.id)
            .collect();
        let mut answer = None;
        let output = ctx.run_ui(raw, |ui| {
            answer = show(ui.ctx(), &self.palette, &self.jobs, &waiting);
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
                        disabled: node.is_disabled(),
                        value: node.numeric_value(),
                    })
                })
                .collect();
        }
        self.painted.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.painted);
        }
        output.drop_without_applying_deltas();
        self.answers.extend(answer);
    }

    fn find(&self, name: &str) -> Option<&Seen> {
        (self.seen.iter())
            .find(|seen| seen.name == name && !matches!(seen.role, Role::Label | Role::TextRun))
    }

    fn get(&self, name: &str) -> &Seen {
        self.find(name).unwrap_or_else(|| {
            let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
            panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
        })
    }

    fn says(&self, text: &str) -> bool {
        self.seen.iter().any(|seen| seen.name.contains(text))
            || self
                .painted
                .iter()
                .any(|(painted, _)| painted.contains(text))
    }

    fn painted_in(&self, text: &str) -> Option<Color32> {
        (self.painted.iter())
            .find(|(painted, _)| painted.contains(text))
            .map(|(_, color)| *color)
    }

    /// Scrolls the list until `name` is in sight, between the header and
    /// the footer.
    fn reveal(&mut self, name: &str) {
        for _ in 0..8 {
            self.frame(Vec::new());
            let top = self.get("Close render queue").rect.bottom();
            // Spec §5.9.4: Clear finished moved into the header; the list
            // ends at the footer's New export….
            let bottom = self
                .find("New export…")
                .map_or(self.size.y, |new| new.rect.top());
            let rect = self.get(name).rect;
            // The header's and the footer's controls do not scroll.
            let chrome = [
                "Close render queue",
                "Clear finished",
                "New export…",
                "Start all",
            ]
            .contains(&name);
            if chrome || (rect.top() >= top && rect.bottom() <= bottom) {
                return;
            }
            let middle = (top + bottom) * 0.5;
            self.frame(vec![Event::PointerMoved(pos2(rect.center().x, middle))]);
            self.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, (middle - rect.center().y).clamp(-400.0, 400.0)),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }]);
            for _ in 0..12 {
                self.frame(Vec::new());
            }
        }
        panic!("FIXTURE: \"{name}\" could not be scrolled into sight");
    }

    fn click(&mut self, name: &str) {
        self.reveal(name);
        let at = self.get(name).rect.center();
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

    fn take(&mut self) -> Vec<QueueAnswer> {
        std::mem::take(&mut self.answers)
    }

    fn id(&self, name: &str) -> Uuid {
        (self.jobs.iter())
            .find(|job| job.name == name)
            .expect("FIXTURE: the job is listed")
            .id
    }
}

fn collect(shape: &Shape, into: &mut Vec<(String, Color32)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, into);
            }
        }
        Shape::Text(text) => {
            let color = (text.galley.job.sections.first())
                .map_or(text.fallback_color, |section| section.format.color);
            let color = if color == Color32::PLACEHOLDER {
                text.fallback_color
            } else {
                color
            };
            into.push((text.galley.text().to_owned(), color));
        }
        _ => {}
    }
}

#[test]
fn each_state_offers_only_the_actions_it_allows() {
    for size in [WIDE, NARROW] {
        let bench = Bench::new(size, every_state());
        let offered = |job: &str| -> Vec<String> {
            let quoted = format!("“{job}”");
            (bench.seen.iter())
                .filter(|seen| seen.role == Role::Button && seen.name.contains(&quoted))
                .map(|seen| seen.name.replace(&quoted, "*"))
                .collect()
        };
        let sorted = |mut names: Vec<String>| {
            names.sort();
            names
        };
        let expect = |names: &[&str]| sorted(names.iter().map(|n| (*n).to_owned()).collect());
        assert_eq!(
            sorted(offered("rendering.mp4")),
            expect(&["Pause *", "Cancel *"]),
            "{size:?}"
        );
        assert_eq!(
            sorted(offered("preparing.mp4")),
            expect(&["Pause *", "Cancel *"])
        );
        assert_eq!(sorted(offered("finalizing.mp4")), expect(&["Cancel *"]));
        assert_eq!(
            sorted(offered("paused.mp4")),
            expect(&["Resume *", "Cancel *"])
        );
        assert_eq!(
            sorted(offered("first.mp4")),
            expect(&["Render * sooner", "Render * later", "Cancel *"])
        );
        assert_eq!(
            sorted(offered("done.mp4")),
            expect(&["Show * in its folder", "Remove * from the list"])
        );
        for job in ["failed.mp4", "cancelled.mp4"] {
            assert_eq!(
                sorted(offered(job)),
                expect(&["Render * again", "Remove * from the list"]),
                "{job}"
            );
        }
        // The first waiting job cannot go sooner, the last not later.
        assert!(bench.get("Render “first.mp4” sooner").disabled);
        assert!(!bench.get("Render “first.mp4” later").disabled);
        assert!(!bench.get("Render “second.mp4” sooner").disabled);
        assert!(bench.get("Render “second.mp4” later").disabled);
        // Every action stays inside the window.
        let close = bench.get("Close render queue").rect;
        assert!(close.right() <= size.x && close.top() >= 0.0);
    }
}

#[test]
fn every_action_answers_with_its_job() {
    let mut bench = Bench::new(WIDE, every_state());
    for (name, job, answer) in [
        (
            "Pause “rendering.mp4”",
            "rendering.mp4",
            QueueAnswer::Pause as fn(Uuid) -> QueueAnswer,
        ),
        (
            "Cancel “rendering.mp4”",
            "rendering.mp4",
            QueueAnswer::Cancel,
        ),
        ("Resume “paused.mp4”", "paused.mp4", QueueAnswer::Resume),
        (
            "Render “second.mp4” sooner",
            "second.mp4",
            QueueAnswer::MoveUp,
        ),
        (
            "Render “first.mp4” later",
            "first.mp4",
            QueueAnswer::MoveDown,
        ),
        ("Cancel “first.mp4”", "first.mp4", QueueAnswer::Cancel),
        (
            "Show “done.mp4” in its folder",
            "done.mp4",
            QueueAnswer::Reveal,
        ),
        (
            "Remove “done.mp4” from the list",
            "done.mp4",
            QueueAnswer::Remove,
        ),
        (
            "Render “failed.mp4” again",
            "failed.mp4",
            QueueAnswer::Retry,
        ),
        (
            "Remove “cancelled.mp4” from the list",
            "cancelled.mp4",
            QueueAnswer::Remove,
        ),
    ] {
        bench.click(name);
        assert_eq!(bench.take(), [answer(bench.id(job))], "{name}");
    }
    bench.click("Render “first.mp4” sooner");
    assert!(bench.take().is_empty(), "a disabled step does nothing");

    bench.click("Clear finished");
    assert_eq!(bench.take(), [QueueAnswer::ClearFinished]);
    bench.click("New export…");
    assert_eq!(bench.take(), [QueueAnswer::NewExport]);
    bench.click("Close render queue");
    assert_eq!(bench.take(), [QueueAnswer::Close]);
    bench.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(bench.take(), [QueueAnswer::Close]);
}

#[test]
fn a_job_says_how_far_it_has_got_and_why_it_failed() {
    let bench = Bench::new(WIDE, every_state());
    let bar = bench.get("Progress of “rendering.mp4”");
    assert_eq!(bar.role, Role::ProgressIndicator);
    assert!((bar.value.unwrap() - 0.34).abs() < 1e-6);
    assert!(bench.find("Progress of “done.mp4”").is_none());
    assert!(bench.find("Progress of “first.mp4”").is_none());

    assert!(bench.says("Frame 120 of 360 · 42.0 fps · 1:02 left"));
    assert!(bench.says("34 %"));
    assert!(bench.says("h264_nvenc"));
    assert!(bench.says("Paused at frame 180 of 360 · 0:42 elapsed"));
    assert!(bench.says("Waiting · MP4 · H.264 · Project size"));
    assert!(bench.says("Done in 1:02 · 45.2 MB"));
    assert!(bench.says("Cancelled at frame 90 of 360"));
    assert!(bench.says("/home/someone/Exports"));
    assert!(
        bench.says("3 rendering · 1 paused · 2 waiting"),
        "the header counts the work"
    );
    let red = bench.palette.red;
    let mut bench = bench;
    // The failed job's message is just over the last row, the job done
    // first.
    bench.reveal("Remove “done.mp4” from the list");
    assert_eq!(bench.painted_in("No space left on device"), Some(red));
    assert_ne!(
        bench.painted_in("rendered in software"),
        Some(red),
        "a note is not a failure"
    );
}

#[test]
fn the_panel_is_in_place_from_its_first_frame() {
    let ctx = egui::Context::default();
    let mut bench = Bench::new(WIDE, Vec::new());
    bench.ctx = ctx;
    theme::install_fonts(&bench.ctx);
    theme::apply(&bench.ctx, &bench.palette);
    bench.ctx.enable_accesskit();
    bench.jobs = every_state();
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let first = bench.get("Close render queue").rect;
    for _ in 0..8 {
        bench.frame(Vec::new());
    }
    assert_eq!(
        bench.get("Close render queue").rect,
        first,
        "it does not creep"
    );
}

#[test]
fn an_empty_queue_says_so_and_offers_an_export() {
    for size in [WIDE, NARROW] {
        let mut bench = Bench::new(size, Vec::new());
        assert!(bench.says("Nothing to render"));
        assert!(bench.find("Clear finished").is_none());
        bench.click("New export…");
        assert_eq!(bench.take(), [QueueAnswer::NewExport]);
    }
}

#[test]
fn nothing_finished_leaves_nothing_to_clear() {
    let mut bench = Bench::new(NARROW, vec![job("first.mp4", RenderState::Queued)]);
    assert!(bench.get("Clear finished").disabled);
    bench.click("Clear finished");
    assert!(bench.take().is_empty());
    // A lone waiting job has no place to move to.
    assert!(bench.get("Render “first.mp4” sooner").disabled);
    assert!(bench.get("Render “first.mp4” later").disabled);
}

#[test]
fn the_ring_draws_the_share_done() {
    let ctx = egui::Context::default();
    let palette = Palette::new(Appearance::Dark, Accent::default());
    theme::install_fonts(&ctx);
    let mut lines = Vec::new();
    for (progress, active) in [(0.0, 1), (0.5, 1), (1.0, 3)] {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(pos2(10.0, 10.0), Vec2::splat(18.0));
            summary_ring(ui, &palette, rect, progress, active);
        });
        let mut texts = Vec::new();
        let mut arcs = 0;
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
            if matches!(clipped.shape, Shape::Path(_)) {
                arcs += 1;
            }
        }
        output.drop_without_applying_deltas();
        lines.push((arcs, texts.into_iter().map(|(t, _)| t).collect::<Vec<_>>()));
    }
    assert_eq!(lines[0], (0, vec![]), "nothing done draws no arc");
    assert_eq!(lines[1], (1, vec![]));
    assert_eq!(
        lines[2],
        (1, vec!["3".to_owned()]),
        "more than one job is counted"
    );
}

#[test]
fn held_jobs_say_so_move_with_the_waiting_and_start_together() {
    // B waits, A was added to the queue held, C waits: the order they
    // render in is B, A, C, and the panel's steps move among all three.
    let jobs = vec![
        job("b.mp4", RenderState::Queued),
        job("a.mp4", RenderState::Paused),
        job("c.mp4", RenderState::Queued),
    ];
    let a = jobs[1].id;
    let c = jobs[2].id;
    let mut bench = Bench::new(WIDE, jobs);
    bench.held = vec![a];
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(
        bench.says("2 waiting · 1 held"),
        "the header names held jobs"
    );
    assert!(!bench.says("1 paused"), "a held job is not called paused");
    assert!(bench.says("Held until you start it · MP4 · H.264 · Project size"));
    assert!(bench.says("Held"));
    // The held job is started, moved and cancelled like a waiting one.
    for name in [
        "Start “a.mp4”",
        "Render “a.mp4” sooner",
        "Render “a.mp4” later",
        "Cancel “a.mp4”",
    ] {
        assert!(!bench.get(name).disabled, "{name}");
    }
    assert!(bench.find("Resume “a.mp4”").is_none());
    // Nothing of it is done: it has no progress bar, the waiting ones none.
    assert!(bench.find("Progress of “a.mp4”").is_none());
    bench.click("Start “a.mp4”");
    assert_eq!(bench.take(), [QueueAnswer::Resume(a)]);
    // C goes one place sooner: before A, which the queue renders before it.
    bench.click("Render “c.mp4” sooner");
    assert_eq!(bench.take(), [QueueAnswer::MoveUp(c)]);
    assert!(!bench.get("Render “c.mp4” sooner").disabled);
    assert!(bench.get("Render “c.mp4” later").disabled);
    bench.click("Start all");
    assert_eq!(bench.take(), [QueueAnswer::StartAll]);

    // A job paused while it rendered is not held: it resumes, and nothing
    // is started all at once.
    let mut paused = job("p.mp4", RenderState::Paused);
    paused.progress = 0.4;
    paused.frames_done = 100;
    let bench = Bench::new(WIDE, vec![paused]);
    assert!(bench.find("Resume “p.mp4”").is_some());
    assert!(bench.find("Start all").is_none());
    assert!(bench.says("1 paused"));
}

#[test]
fn a_folder_is_shown_by_its_last_folders_from_home() {
    let home = Path::new("/home/someone");
    let short = |path: &str| short_folder(Path::new(path), Some(home));
    let sep = MAIN_SEPARATOR;
    if sep == '/' {
        assert_eq!(short("/home/someone"), "Home folder");
        assert_eq!(short("/home/someone/Videos"), "~/Videos");
        assert_eq!(short("/home/someone/Videos/Trip"), "~/Videos/Trip");
        assert_eq!(
            short("/home/someone/Documents/Projects/editor/artifacts/out"),
            "~/…/artifacts/out"
        );
        assert_eq!(short("/media/disk"), "/media/disk");
        assert_eq!(short("/media/disk/films/2026/out"), "/…/2026/out");
        assert_eq!(short_folder(Path::new("/srv/a/b/c"), None), "/…/b/c");
    }
    // The row shows the short form and keeps the whole path for hover.
    let mut deep = job("deep.mp4", RenderState::Done);
    deep.destination = PathBuf::from("/srv/films/2026/october/out/deep.mp4");
    let bench = Bench::new(WIDE, vec![deep]);
    if sep == '/' {
        assert!(bench.says("/…/october/out"));
        assert!(
            !(bench.painted.iter()).any(|(text, _)| text.contains("/srv/films/2026")),
            "the whole path is not painted"
        );
    }
    assert!(bench.says("Saved in /srv/films/2026/october/out/deep.mp4") || sep != '/');
    // The folder begins where the status under it does, after its glyph.
    let left = |start: &str| {
        (bench.seen.iter())
            .find(|seen| seen.name.starts_with(start))
            .map(|seen| seen.rect.left())
            .unwrap_or_else(|| panic!("FIXTURE: nothing says \"{start}\""))
    };
    let (folder, status) = (left("Saved in "), left("Done in "));
    assert!(
        (folder - status).abs() <= 1.0,
        "the folder at {folder} lines up with the status at {status}"
    );
}

/// Where the row of `job` begins: the top of its name.
fn row_top(bench: &Bench, job: &str) -> f32 {
    (bench.seen.iter())
        .find(|seen| seen.name == job && seen.role == Role::Label)
        .unwrap_or_else(|| panic!("FIXTURE: the row of {job} is listed"))
        .rect
        .top()
}

/// Whether `name` is shown whole between the header and the footer.
fn in_sight(bench: &Bench, name: &str) -> bool {
    let top = bench.get("Close render queue").rect.bottom();
    // Spec §5.9.4: Clear finished moved into the header; the list ends at
    // the footer's New export….
    let bottom = bench.get("New export…").rect.top();
    let rect = bench.get(name).rect;
    rect.top() >= top && rect.bottom() <= bottom
}

/// Done jobs, named `done-0.mp4` on, listed as the queue lists them.
fn finished(count: usize) -> Vec<RenderJobInfo> {
    (0..count)
        .map(|at| {
            let mut done = job(&format!("done-{at}.mp4"), RenderState::Done);
            done.progress = 1.0;
            done.output_bytes = Some(1_000_000);
            done
        })
        .collect()
}

#[test]
fn the_job_under_way_comes_first_then_the_waiting_then_the_finished() {
    // As the queue lists them, in render order: six done jobs, the one
    // rendering, one waiting and one held.
    let mut jobs = finished(6);
    let mut rendering = job("rendering.mp4", RenderState::Rendering);
    rendering.progress = 0.3;
    jobs.push(rendering);
    jobs.push(job("next.mp4", RenderState::Queued));
    jobs.push(job("held.mp4", RenderState::Paused));
    let held = jobs[8].id;
    let mut bench = Bench::new(WIDE, jobs);
    bench.held = vec![held];
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(
        in_sight(&bench, "Pause “rendering.mp4”"),
        "the running job is in sight as the panel opens"
    );
    let order: Vec<f32> = [
        "rendering.mp4",
        "next.mp4",
        "held.mp4",
        "done-5.mp4",
        "done-4.mp4",
        "done-0.mp4",
    ]
    .iter()
    .map(|name| row_top(&bench, name))
    .collect();
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "under way, waiting and held in render order, then the most recent \
         finished first: {order:?}"
    );
    // The steps still move among the waiting jobs, held ones included.
    assert!(bench.get("Render “next.mp4” sooner").disabled);
    assert!(!bench.get("Render “next.mp4” later").disabled);
    assert!(!bench.get("Render “held.mp4” sooner").disabled);

    // The running job finishes and the next starts: the one just done is
    // the first of the finished, the new one is first of all.
    bench.jobs[6].state = RenderState::Done;
    bench.jobs[7].state = RenderState::Rendering;
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(row_top(&bench, "next.mp4") < row_top(&bench, "held.mp4"));
    assert!(row_top(&bench, "held.mp4") < row_top(&bench, "rendering.mp4"));
    assert!(row_top(&bench, "rendering.mp4") < row_top(&bench, "done-5.mp4"));
}

#[test]
fn a_job_retried_and_done_again_is_the_most_recent() {
    let mut jobs = finished(3);
    jobs[0].state = RenderState::Failed;
    let mut bench = Bench::new(WIDE, jobs);
    assert!(row_top(&bench, "done-0.mp4") > row_top(&bench, "done-2.mp4"));
    // Retried: it waits, renders, then is done, after the others.
    for state in [
        RenderState::Queued,
        RenderState::Rendering,
        RenderState::Done,
    ] {
        bench.jobs[0].state = state;
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
    }
    assert!(
        row_top(&bench, "done-0.mp4") < row_top(&bench, "done-2.mp4"),
        "the job done last is listed first among the finished"
    );
}

#[test]
fn a_job_that_starts_is_scrolled_into_sight_once() {
    // A short window: the list scrolls. The person has scrolled to the
    // last finished job when the waiting one starts.
    let mut jobs = vec![job("next.mp4", RenderState::Queued)];
    jobs.extend(finished(8));
    let mut bench = Bench::new(NARROW, jobs);
    bench.reveal("Remove “done-0.mp4” from the list");
    assert!(
        !in_sight(&bench, "Cancel “next.mp4”"),
        "FIXTURE: scrolled away"
    );
    bench.jobs[0].state = RenderState::Rendering;
    for _ in 0..12 {
        bench.frame(Vec::new());
    }
    assert!(
        in_sight(&bench, "Pause “next.mp4”"),
        "it is brought into sight"
    );
    // Once: the person may scroll away from it again.
    bench.reveal("Remove “done-0.mp4” from the list");
    for _ in 0..12 {
        bench.frame(Vec::new());
    }
    assert!(
        !in_sight(&bench, "Pause “next.mp4”"),
        "it is not held in sight"
    );
}
