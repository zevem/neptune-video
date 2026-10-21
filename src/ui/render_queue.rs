//! The render queue panel: every export queued, rendering or finished, and
//! what can be done with each. It reads the queue's snapshot and answers what
//! the person asked; the application tells the queue. It asks for no repaint
//! of its own beyond its scrim's fade as it opens: the queue wakes the window
//! while a job renders.
//!
//! It is drawn on the export sheet's scaffold, as tall in every state so that
//! nothing in it moves as jobs come and go: a 52-point header with Clear
//! finished beside the close control, the jobs as one card of hairline rows,
//! and a fixed footer.
use super::{
    controls::{self, ButtonKind},
    export_sheet::{header, refusal_line, scaffold, scrolled},
    kit,
};
use crate::{
    icons::{self, Icon},
    theme::{self, Accent, Palette},
    timefmt,
};
use editor_core::{
    Time,
    export::{ExportSettings, RenderJobInfo, RenderState, Resolution},
};
use eframe::egui::{
    self, Align, Align2, Id, Layout, Rect, Sense, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use std::{
    collections::HashMap,
    path::{Component, MAIN_SEPARATOR, Path},
};
use uuid::Uuid;

/// The sheet at its widest and tallest. A narrower window keeps 12 points
/// clear of it on each side, and a shorter one keeps its title bar clear,
/// as Preferences does (`sheets::sheet_height`).
const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 480.0;
const HEADER: f32 = 52.0;
const FOOTER: f32 = 60.0;
/// The sheet's margin.
const PAD: f32 = 20.0;
/// The summary's line under the title.
const SUMMARY: f32 = 22.0;
/// Where a row's words begin: after its 64-point poster and 12 on either
/// side of it.
const TEXT: f32 = 12.0 + 64.0 + 12.0;
/// A row's padding over and under what it holds.
const ROW_PAD: f32 = 8.0;
/// The lines of a row: its name and actions, its status, its progress and
/// its folder.
const NAME_LINE: f32 = 22.0;
const LINE: f32 = 18.0;
const BAR_LINE: f32 = 14.0;

/// What the person asked of the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueAnswer {
    Close,
    Pause(Uuid),
    /// Resume a paused job, or start one held since it was queued.
    Resume(Uuid),
    /// Start every held job, in the order listed.
    StartAll,
    Cancel(Uuid),
    /// Render a failed or cancelled job again from its snapshot.
    Retry(Uuid),
    /// Take a finished job off the list; its file stays.
    Remove(Uuid),
    /// Show the finished file in the system's file manager.
    Reveal(Uuid),
    /// Render a waiting job one place sooner or later.
    MoveUp(Uuid),
    MoveDown(Uuid),
    ClearFinished,
    NewExport,
}

/// Shows the render queue. `waiting` is the queue's waiting jobs in the
/// order they render, held ones included: a waiting job that reads Paused is
/// held, queued by Add to queue and started only when the person says so.
/// `refusal` is why what was last asked of the queue failed, said at the
/// footer's leading edge. Returns what the person asked for, if anything;
/// Escape and the backdrop close it.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    jobs: &[RenderJobInfo],
    waiting: &[Uuid],
    refusal: Option<&str>,
) -> Option<QueueAnswer> {
    let mut answer = None;
    let home = directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    let screen = ctx.content_rect();
    let width = WIDTH.min(screen.width() - 24.0).max(240.0);
    let most = super::sheets::sheet_height(HEIGHT, screen.height());
    let id = Id::new("render-queue-sheet");
    let response = scaffold(ctx, p, id, "Render queue", width, |ui| {
        let finished = jobs.iter().any(|job| job.state.is_finished());
        if header(ui, p, "Render queue", "Close render queue", |ui| {
            // Nothing is listed to clear in an empty queue.
            if jobs.is_empty() {
                return;
            }
            let clear = ui
                .add_enabled_ui(finished, |ui| {
                    controls::button(ui, *p, "Clear finished", ButtonKind::Quiet)
                })
                .inner
                .on_hover_text("Take finished, failed and cancelled jobs off the list")
                .on_disabled_hover_text("No job has finished");
            if clear.clicked() && finished {
                answer = Some(QueueAnswer::ClearFinished);
            }
        }) {
            answer = Some(QueueAnswer::Close);
        }

        if jobs.is_empty() {
            let (_, rect) = ui.allocate_space(vec2(width, most - HEADER));
            if kit::empty_state(
                ui,
                p,
                rect,
                Icon::List,
                "Nothing to render",
                "Exports wait here and render one at a time in the background while you keep \
                 editing.",
                Some("New export…"),
            ) {
                answer = Some(QueueAnswer::NewExport);
            }
            return;
        }

        // With nothing under way or waiting there is nothing to count, and
        // its line stays, so that the list does not move up.
        let summary = summary(jobs, waiting);
        let (_, line) = ui.allocate_space(vec2(width, SUMMARY));
        if !summary.is_empty() {
            let words = ui.painter().text(
                pos2(line.left() + PAD, line.top() - 4.0),
                Align2::LEFT_TOP,
                &summary,
                theme::regular(12.0),
                p.secondary,
            );
            kit::said(ui, words, &summary);
        }

        let order_id = Id::new("render-queue-order");
        let mut order: Order = ui.data_mut(|data| data.get_temp(order_id).unwrap_or_default());
        let rows = order.listed(jobs, waiting);
        // A job that has just started is scrolled into sight once.
        let under_way = rows
            .first()
            .filter(|job| under_way(job, waiting))
            .map(|job| job.id);
        let reveal = under_way.filter(|id| order.shown_under_way != Some(*id));
        order.shown_under_way = under_way;
        ui.data_mut(|data| data.insert_temp(order_id, order));

        // Every row's height is known before it is drawn, so that a row is
        // as tall as it will be from its first frame and does not creep.
        let card = width - PAD * 2.0;
        let heights: Vec<f32> = (rows.iter())
            .map(|job| row_height(ui, job, waiting, card))
            .collect();
        let (_, list) =
            ui.allocate_space(vec2(width, (most - HEADER - SUMMARY - FOOTER).max(80.0)));
        let area = Rect::from_min_max(
            pos2(list.left() + PAD, list.top()),
            pos2(list.right() - 6.0, list.bottom()),
        );
        // The list stays where it was left: a job under way is scrolled to.
        let salt = "render-queue-jobs";
        scrolled(ui, p.elevated, area, card, salt, false, |ui| {
            controls::group(ui, *p, |ui, group| {
                for (job, height) in rows.iter().zip(&heights) {
                    let place = waiting
                        .iter()
                        .position(|id| *id == job.id)
                        .map(|at| (at, waiting.len()));
                    let rect = group.custom(ui, *height);
                    if let Some(asked) = job_row(ui, p, rect, job, place, home.as_deref()) {
                        answer = Some(asked);
                    }
                    if reveal == Some(job.id) {
                        // With the room over it, so that the first row
                        // shows the card's top edge as well.
                        ui.scroll_to_rect(rect.expand2(vec2(0.0, 4.0)), Some(Align::Min));
                    }
                }
            });
        });

        let bar = controls::sheet_footer(ui, *p).shrink2(vec2(PAD - 16.0, 0.0));
        controls::place(
            ui,
            bar,
            Layout::right_to_left(Align::Center),
            "render-queue-footer",
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if controls::button(ui, *p, "New export…", ButtonKind::Secondary).clicked() {
                    answer = Some(QueueAnswer::NewExport);
                }
                // Held jobs start only when asked: all of them at once.
                let held = (jobs.iter()).filter(|job| is_held(job, waiting)).count();
                if held > 0
                    && controls::button(ui, *p, "Start all", ButtonKind::Secondary)
                        .on_hover_text(match held {
                            1 => "Start the held job".to_owned(),
                            n => format!("Start the {n} held jobs, in the order listed"),
                        })
                        .clicked()
                {
                    answer = Some(QueueAnswer::StartAll);
                }
                // What is left at the leading edge says why the last
                // request failed.
                if let Some(refusal) = refusal {
                    let room = Rect::from_min_max(
                        bar.left_top(),
                        pos2(bar.left() + ui.available_width() - 8.0, bar.bottom()),
                    );
                    refusal_line(ui, p, room, refusal);
                }
            },
        );
    });
    if answer.is_none() && response.should_close() {
        answer = Some(QueueAnswer::Close);
    }
    answer
}

/// The order the panel lists jobs in, kept between frames: when each
/// finished job was first seen finished, and the job under way the list was
/// last scrolled to.
#[derive(Clone, Debug, Default)]
struct Order {
    /// Finished jobs by when they were first seen finished, later larger.
    finished: HashMap<Uuid, u64>,
    next: u64,
    shown_under_way: Option<Uuid>,
}

impl Order {
    /// The jobs as listed: the one under way first (rendering, or started
    /// and paused), then the waiting ones in the order they render, held
    /// ones included, then any other unfinished one, and the finished ones
    /// last, the most recent first. Jobs found finished together, as when
    /// the panel is opened, are taken to have finished in the queue's
    /// order, which is the order they render in.
    fn listed<'a>(
        &mut self,
        jobs: &'a [RenderJobInfo],
        waiting: &[Uuid],
    ) -> Vec<&'a RenderJobInfo> {
        // A job retried or taken off the list is forgotten: when it
        // finishes again it is the most recent.
        self.finished
            .retain(|id, _| (jobs.iter()).any(|job| job.id == *id && job.state.is_finished()));
        for job in jobs.iter().filter(|job| job.state.is_finished()) {
            if !self.finished.contains_key(&job.id) {
                self.finished.insert(job.id, self.next);
                self.next += 1;
            }
        }
        let mut rows: Vec<&RenderJobInfo> = (jobs.iter())
            .filter(|job| under_way(job, waiting))
            .collect();
        rows.extend(
            waiting
                .iter()
                .filter_map(|id| jobs.iter().find(|job| job.id == *id)),
        );
        rows.extend(jobs.iter().filter(|job| {
            !job.state.is_finished() && !under_way(job, waiting) && !waiting.contains(&job.id)
        }));
        let mut finished: Vec<&RenderJobInfo> = (jobs.iter())
            .filter(|job| job.state.is_finished())
            .collect();
        finished.sort_by_key(|job| std::cmp::Reverse(self.finished.get(&job.id).copied()));
        rows.extend(finished);
        rows
    }
}

/// A job that has started and not finished: rendering, or paused after it
/// started.
fn under_way(job: &RenderJobInfo, waiting: &[Uuid]) -> bool {
    job.state.is_active() || (job.state == RenderState::Paused && !waiting.contains(&job.id))
}

/// A waiting job that reads Paused: queued held, by Add to queue or paused
/// before its turn, and started only when the person says so.
fn is_held(job: &RenderJobInfo, waiting: &[Uuid]) -> bool {
    job.state == RenderState::Paused && waiting.contains(&job.id)
}

/// "1 rendering · 2 waiting · 1 held".
fn summary(jobs: &[RenderJobInfo], waiting: &[Uuid]) -> String {
    let count = |wanted: &dyn Fn(&RenderJobInfo) -> bool| jobs.iter().filter(|j| wanted(j)).count();
    let active = count(&|job| job.state.is_active());
    let held = count(&|job| is_held(job, waiting));
    let paused = count(&|job| job.state == RenderState::Paused) - held;
    let queued = count(&|job| job.state == RenderState::Queued);
    let mut parts = Vec::new();
    if active > 0 {
        parts.push(format!("{active} rendering"));
    }
    if paused > 0 {
        parts.push(format!("{paused} paused"));
    }
    if queued > 0 {
        parts.push(format!("{queued} waiting"));
    }
    if held > 0 {
        parts.push(format!("{held} held"));
    }
    parts.join(" · ")
}

/// The folder a job writes into, short enough for its row: the home folder
/// as `~` ("Home folder" when it is the folder), and only the last two
/// folders of a deeper path, so that two jobs are told apart by where they
/// go: `~/…/Trip/Exports`. The export sheet names its folder the same way.
pub(super) fn short_folder(folder: &Path, home: Option<&Path>) -> String {
    let sep = MAIN_SEPARATOR;
    let names = |path: &Path| -> Vec<String> {
        (path.components())
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    if let Some(home) = home
        && let Ok(inside) = folder.strip_prefix(home)
    {
        let names = names(inside);
        return match names.len() {
            // A lone `~` reads as a stray mark.
            0 => "Home folder".to_owned(),
            1 | 2 => format!("~{sep}{}", names.join(&sep.to_string())),
            n => format!("~{sep}…{sep}{}", names[n - 2..].join(&sep.to_string())),
        };
    }
    let all = names(folder);
    if all.len() <= 3 {
        return folder.display().to_string();
    }
    // The root as written (`/`, `C:\`), then the last two folders.
    let root: String = (folder.components())
        .take_while(|part| !matches!(part, Component::Normal(_)))
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let root = if root.ends_with(sep) || root.is_empty() {
        root
    } else {
        format!("{root}{sep}")
    };
    format!(
        "{root}…{sep}{}",
        all[all.len() - 2..].join(&sep.to_string())
    )
}

/// `short_folder` cut to what `fits`: the folders over the last go first,
/// leaving `~/…/Exports`, then the middle of the last folder's own name, so
/// that its start and its end, which tell one folder from the next, stay
/// seen. Whatever still does not fit is cut short at its end where it is
/// painted.
pub(super) fn fitted_folder(
    folder: &Path,
    home: Option<&Path>,
    fits: impl Fn(&str) -> bool,
) -> String {
    let short = short_folder(folder, home);
    if fits(&short) {
        return short;
    }
    let sep = MAIN_SEPARATOR;
    let inside = home.and_then(|home| folder.strip_prefix(home).ok());
    let Some(last) = (inside.unwrap_or(folder).components())
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .next_back()
    else {
        return short;
    };
    // What stands before the last folder: `~` or the root as written, and
    // `…` for any folders between.
    let lead = match inside {
        Some(_) => format!("~{sep}"),
        None => {
            let root: String = (folder.components())
                .take_while(|part| !matches!(part, Component::Normal(_)))
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            if root.ends_with(sep) || root.is_empty() {
                root
            } else {
                format!("{root}{sep}")
            }
        }
    };
    let lead = if short == format!("{lead}{last}") {
        lead
    } else {
        format!("{lead}…{sep}")
    };
    let whole = format!("{lead}{last}");
    if fits(&whole) {
        return whole;
    }
    // The most of the name's start and end that fits, found by halves.
    let chars: Vec<char> = last.chars().collect();
    let cut = |keep: usize| -> String {
        let head: String = chars[..keep.div_ceil(2)].iter().collect();
        let tail: String = chars[chars.len() - keep / 2..].iter().collect();
        format!("{lead}{head}…{tail}")
    };
    let (mut low, mut high) = (1, chars.len().saturating_sub(1));
    if low > high || !fits(&cut(low)) {
        return whole;
    }
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(&cut(mid)) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    cut(low)
}

/// What a job makes, in a few words: "MP4 · H.264 · 1080p".
fn made(settings: &ExportSettings) -> String {
    let mut parts = vec![settings.container.label().to_owned()];
    match &settings.video {
        Some(video) => {
            parts.push(video.codec.label().to_owned());
            parts.push(match video.resolution {
                Resolution::Project => "Project size".to_owned(),
                Resolution::Short { pixels: 2160 } => "4K".to_owned(),
                Resolution::Short { pixels } => format!("{pixels}p"),
                Resolution::Custom { width, height } => format!("{width} × {height}"),
            });
        }
        None => {
            if let Some(audio) = &settings.audio {
                parts.push(audio.codec.label().to_owned());
            }
        }
    }
    parts.join(" · ")
}

fn duration(seconds: f64) -> String {
    timefmt::short(Time::from_seconds(seconds.max(0.0)))
}

/// What the job is doing, in a line under its bar. A held job waits for the
/// person to start it.
fn status(job: &RenderJobInfo, held: bool) -> String {
    if held {
        return format!("Held until you start it · {}", made(&job.settings));
    }
    let frames = if job.frames_total > 0 {
        format!("frame {} of {}", job.frames_done, job.frames_total)
    } else {
        String::new()
    };
    match job.state {
        RenderState::Queued => format!("Waiting · {}", made(&job.settings)),
        RenderState::Preparing => "Preparing the sound and the encoder…".to_owned(),
        RenderState::Rendering => {
            let mut parts = Vec::new();
            if !frames.is_empty() {
                parts.push(capitalized(&frames));
            }
            if job.speed_fps > 0.0 {
                parts.push(format!("{:.1} fps", job.speed_fps));
            }
            if let Some(eta) = job.eta_seconds {
                parts.push(format!("{} left", duration(eta)));
            }
            if parts.is_empty() {
                "Rendering…".to_owned()
            } else {
                parts.join(" · ")
            }
        }
        RenderState::Paused => {
            if frames.is_empty() {
                format!("Paused · {}", made(&job.settings))
            } else {
                format!(
                    "Paused at {frames} · {} elapsed",
                    duration(job.elapsed_seconds)
                )
            }
        }
        RenderState::Finalizing => "Checking the file and moving it into place…".to_owned(),
        RenderState::Done => match job.output_bytes {
            Some(bytes) => format!(
                "Done in {} · {}",
                duration(job.elapsed_seconds),
                timefmt::file_size(bytes)
            ),
            None => format!("Done in {}", duration(job.elapsed_seconds)),
        },
        RenderState::Failed => format!("Failed after {}", duration(job.elapsed_seconds)),
        RenderState::Cancelled if !frames.is_empty() && job.frames_done > 0 => {
            format!("Cancelled at {frames}")
        }
        RenderState::Cancelled => "Cancelled".to_owned(),
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Whether a job shows how far it has got: started and not finished. A
/// held job has not started.
fn moving(job: &RenderJobInfo, held: bool) -> bool {
    !held
        && matches!(
            job.state,
            RenderState::Preparing
                | RenderState::Rendering
                | RenderState::Paused
                | RenderState::Finalizing
        )
}

/// The folder a job's file goes in.
fn folder_of(job: &RenderJobInfo) -> Option<&Path> {
    (job.destination.parent()).filter(|folder| !folder.as_os_str().is_empty())
}

/// What the queue says of a job besides its state, such as why it failed.
fn message_of(job: &RenderJobInfo) -> Option<&str> {
    job.message.as_deref().filter(|m| !m.trim().is_empty())
}

/// The message laid out for a row whose card is `card` wide: two lines at
/// most, cut short where it runs on, so that a long FFmpeg error does not
/// push the other jobs out of sight. The whole of it is on hover.
fn message_galley(
    ui: &Ui,
    p: &Palette,
    job: &RenderJobInfo,
    card: f32,
) -> Option<std::sync::Arc<egui::Galley>> {
    let message = message_of(job)?;
    let color = if job.state == RenderState::Failed {
        p.red
    } else {
        p.secondary
    };
    let mut layout =
        egui::text::LayoutJob::simple(message.to_owned(), theme::regular(12.0), color, 0.0);
    layout.wrap = egui::text::TextWrapping {
        max_width: (card - TEXT - 12.0).max(40.0),
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    Some(ui.painter().layout_job(layout))
}

/// How tall the row of `job` is in a card `card` wide: its name and status,
/// then the lines its state has.
fn row_height(ui: &Ui, job: &RenderJobInfo, waiting: &[Uuid], card: f32) -> f32 {
    let held = is_held(job, waiting);
    let mut height = ROW_PAD + NAME_LINE + LINE;
    if moving(job, held) {
        height += BAR_LINE;
    }
    if folder_of(job).is_some() {
        height += LINE;
    }
    // Its colour does not change its height.
    let p = Palette::stage_palette(Accent::default());
    if let Some(galley) = message_galley(ui, &p, job, card) {
        height += 2.0 + galley.size().y;
    }
    (height + ROW_PAD).max(56.0)
}

/// One job, in `rect`: its poster, its name and the actions its state
/// allows, then what it is doing, how far it has got, where it goes, and
/// what the queue says of it. `place` is its place among waiting jobs, held
/// ones included, and how many wait; a paused job with a place is held.
fn job_row(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    job: &RenderJobInfo,
    place: Option<(usize, usize)>,
    home: Option<&Path>,
) -> Option<QueueAnswer> {
    let mut answer = None;
    let name = job.name.as_str();
    let held = job.state == RenderState::Paused && place.is_some();
    let painter = ui.painter().clone();
    let card = rect.width();

    // The poster: what the job makes, on the stage.
    let poster = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.top() + ROW_PAD + 2.0),
        vec2(64.0, 36.0),
    );
    if ui.is_rect_visible(poster) {
        let stage = Palette::stage_palette(Accent::default());
        painter.rect_filled(poster, 6, p.stage());
        let glyph = if job.settings.video.is_none() {
            Icon::Music
        } else {
            Icon::Film
        };
        kit::on_dark(&painter, || {
            icons::paint(
                &painter,
                Rect::from_center_size(poster.center(), Vec2::splat(16.0)),
                glyph,
                stage.muted,
            );
        });
    }

    let left = rect.left() + TEXT;
    let right = rect.right() - 12.0;
    let mut y = rect.top() + ROW_PAD;

    // The actions, at the trailing end of the name's line, always shown.
    let line = Rect::from_min_max(pos2(left, y), pos2(right, y + NAME_LINE));
    let actions = controls::place(
        ui,
        line,
        Layout::right_to_left(Align::Center),
        ("render-queue-actions", job.id),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut act = |ui: &mut Ui, icon, label: String, enabled: bool, asked| {
                let clicked = ui
                    .add_enabled_ui(enabled, |ui| kit::small_icon_button(ui, p, icon, &label))
                    .inner
                    .clicked();
                if clicked && enabled {
                    answer = Some(asked);
                }
            };
            actions(&mut act, ui, job, name, held, place);
            ui.min_rect().left()
        },
    );

    // The name, cut short before the actions.
    let room = (actions - 8.0 - left).max(0.0);
    let galley = controls::elided(&painter, name, theme::medium(13.0), p.fg, room);
    let cut = galley.text() != name;
    let words = Rect::from_min_size(
        pos2(left, line.center().y - galley.size().y * 0.5),
        galley.size(),
    );
    painter.galley(words.min, galley, p.fg);
    let named = ui.interact(words, ui.id().with(("job-name", job.id)), Sense::hover());
    named.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
    if cut {
        named.on_hover_text(name);
    }
    y += NAME_LINE;

    // What it is doing: done with a check, failed in the problem colour,
    // and the encoder at the trailing end.
    let status = status(job, held);
    let line = Rect::from_min_max(pos2(left, y), pos2(right, y + LINE));
    let mut x = left;
    if job.state == RenderState::Done {
        icons::paint(
            &painter,
            Rect::from_center_size(pos2(x + 7.0, line.center().y), Vec2::splat(14.0)),
            Icon::CircleCheck,
            p.green,
        );
        x += 14.0 + 5.0;
    }
    let mut end = right;
    let mut encoder_shown = None;
    if let Some(encoder) = &job.encoder {
        let galley = painter.layout_no_wrap(encoder.clone(), theme::mono(11.0), p.secondary);
        end -= galley.size().x;
        let at = pos2(end, line.center().y - galley.size().y * 0.5);
        let shown = Rect::from_min_size(at, galley.size());
        painter.galley(at, galley, p.secondary);
        encoder_shown = Some((shown, encoder));
        end -= 12.0;
    }
    let ink = if job.state == RenderState::Failed {
        p.red
    } else {
        p.secondary
    };
    let galley = controls::elided(&painter, &status, theme::regular(12.0), ink, end - x);
    let stated = Rect::from_min_max(
        pos2(left, line.center().y - galley.size().y * 0.5),
        pos2(x + galley.size().x, line.center().y + galley.size().y * 0.5),
    );
    painter.galley(pos2(x, stated.top()), galley, ink);
    ui.interact(stated, ui.id().with(("job-status", job.id)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &status));
    // The encoder is read after the status, as it is seen.
    if let Some((shown, encoder)) = encoder_shown {
        kit::said(ui, shown, encoder);
    }
    y += LINE;

    // How far it has got.
    if moving(job, held) {
        let line = Rect::from_min_max(pos2(left, y), pos2(right, y + BAR_LINE));
        let percent = format!("{:.0} %", (job.progress.clamp(0.0, 1.0) * 100.0).floor());
        let galley = painter.layout_no_wrap(percent, theme::mono(11.0), p.secondary);
        let bar = Rect::from_min_max(
            pos2(left, line.center().y - 2.0),
            pos2(right - 40.0, line.center().y + 2.0),
        );
        painter.galley(
            pos2(
                right - galley.size().x,
                line.center().y - galley.size().y * 0.5,
            ),
            galley,
            p.secondary,
        );
        let fill = if job.state == RenderState::Paused {
            p.secondary
        } else {
            p.accent
        };
        controls::place(
            ui,
            bar,
            Layout::left_to_right(Align::Center),
            ("render-queue-progress", job.id),
            |ui| {
                kit::progress_bar(
                    ui,
                    p,
                    Some(job.progress),
                    fill,
                    &format!("Progress of “{name}”"),
                )
            },
        );
        y += BAR_LINE;
    }

    // Where it goes, under its status: the last folders, which tell jobs
    // apart, with the whole path on hover.
    if let Some(folder) = folder_of(job) {
        let line = Rect::from_min_max(pos2(left, y), pos2(right, y + LINE));
        icons::paint(
            &painter,
            Rect::from_center_size(pos2(left + 7.0, line.center().y), Vec2::splat(12.0)),
            Icon::Folder,
            p.muted,
        );
        let room = right - left - 19.0;
        let font = theme::regular(11.5);
        let folder = fitted_folder(folder, home, |text| {
            let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), p.secondary);
            galley.size().x <= room
        });
        let galley = controls::elided(&painter, &folder, font, p.secondary, room);
        let shown = Rect::from_min_max(
            pos2(left, line.center().y - galley.size().y * 0.5),
            pos2(
                left + 19.0 + galley.size().x,
                line.center().y + galley.size().y * 0.5,
            ),
        );
        painter.galley(pos2(left + 19.0, shown.top()), galley, p.secondary);
        let saved = format!("Saved in {}", job.destination.display());
        let response = ui.interact(shown, ui.id().with(("job-folder", job.id)), Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &saved));
        response.on_hover_text(job.destination.display().to_string());
        y += LINE;
    }

    // What the queue says of it: why it failed, or a note.
    if let Some(galley) = message_galley(ui, p, job, card) {
        let at = pos2(left, y + 2.0);
        let shown = Rect::from_min_size(at, galley.size());
        let message = galley.text().to_owned();
        let color = if job.state == RenderState::Failed {
            p.red
        } else {
            p.secondary
        };
        let cut = galley.elided;
        painter.galley(at, galley, color);
        let response = ui.interact(shown, ui.id().with(("job-message", job.id)), Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &message));
        if cut {
            response.on_hover_text(message);
        }
    }
    answer
}

/// The actions a job's state allows, trailing first, as `act` makes them.
fn actions(
    act: &mut impl FnMut(&mut Ui, Icon, String, bool, QueueAnswer),
    ui: &mut Ui,
    job: &RenderJobInfo,
    name: &str,
    held: bool,
    place: Option<(usize, usize)>,
) {
    let id = job.id;
    match job.state {
        RenderState::Done => {
            act(
                ui,
                Icon::Trash,
                format!("Remove “{name}” from the list"),
                true,
                QueueAnswer::Remove(id),
            );
            act(
                ui,
                Icon::Folder,
                format!("Show “{name}” in its folder"),
                true,
                QueueAnswer::Reveal(id),
            );
        }
        RenderState::Failed | RenderState::Cancelled => {
            act(
                ui,
                Icon::Trash,
                format!("Remove “{name}” from the list"),
                true,
                QueueAnswer::Remove(id),
            );
            act(
                ui,
                Icon::Loop,
                format!("Render “{name}” again"),
                true,
                QueueAnswer::Retry(id),
            );
        }
        // Waiting its turn, or held until started.
        RenderState::Queued | RenderState::Paused if held || job.state == RenderState::Queued => {
            act(
                ui,
                Icon::Close,
                format!("Cancel “{name}”"),
                true,
                QueueAnswer::Cancel(id),
            );
            if held {
                act(
                    ui,
                    Icon::Play,
                    format!("Start “{name}”"),
                    true,
                    QueueAnswer::Resume(id),
                );
            }
            if let Some((at, of)) = place {
                act(
                    ui,
                    Icon::ArrowDown,
                    format!("Render “{name}” later"),
                    at + 1 < of,
                    QueueAnswer::MoveDown(id),
                );
                act(
                    ui,
                    Icon::ArrowUp,
                    format!("Render “{name}” sooner"),
                    at > 0,
                    QueueAnswer::MoveUp(id),
                );
            }
        }
        // Started, then paused.
        RenderState::Queued | RenderState::Paused => {
            act(
                ui,
                Icon::Stop,
                format!("Cancel “{name}”"),
                true,
                QueueAnswer::Cancel(id),
            );
            act(
                ui,
                Icon::Play,
                format!("Resume “{name}”"),
                true,
                QueueAnswer::Resume(id),
            );
        }
        RenderState::Preparing | RenderState::Rendering => {
            act(
                ui,
                Icon::Stop,
                format!("Cancel “{name}”"),
                true,
                QueueAnswer::Cancel(id),
            );
            act(
                ui,
                Icon::Pause,
                format!("Pause “{name}”"),
                true,
                QueueAnswer::Pause(id),
            );
        }
        RenderState::Finalizing => {
            act(
                ui,
                Icon::Stop,
                format!("Cancel “{name}”"),
                true,
                QueueAnswer::Cancel(id),
            );
        }
    }
}

/// A small ring for the toolbar's queue button: the share of the work done
/// as an arc from the top, and the number of jobs under way when more than
/// one is. Painted only; the button around it carries the name.
pub fn summary_ring(ui: &Ui, p: &Palette, rect: Rect, progress: f32, active: usize) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let center = rect.center();
    kit::progress_ring(painter, p, center, progress);
    if active > 1 {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            active.min(99).to_string(),
            theme::mono(11.0),
            p.fg,
        );
    }
}

#[cfg(test)]
#[path = "render_queue_tests.rs"]
mod tests;
