//! The render queue panel: every export queued, rendering or finished, and
//! what can be done with each. It reads the queue's snapshot and answers what
//! the person asked; the application tells the queue. It asks for no repaint
//! of its own: the queue wakes the window while a job renders.
use super::{
    sheets,
    widgets::{self, Kind, icon_button},
};
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
    timefmt,
};
use editor_core::{
    Time,
    export::{ExportSettings, RenderJobInfo, RenderState, Resolution},
};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, Layout, Rect, RichText, Sense, Stroke, Ui,
    Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use std::{
    collections::HashMap,
    f32::consts::TAU,
    path::{Component, MAIN_SEPARATOR, Path},
};
use uuid::Uuid;

/// About how tall a job's row is, for the first frame the queue is shown.
const ROW_ESTIMATE: f32 = 96.0;

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
/// Returns what the person asked for, if anything; Escape and the backdrop
/// close it.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    jobs: &[RenderJobInfo],
    waiting: &[Uuid],
) -> Option<QueueAnswer> {
    let mut answer = None;
    let home = directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    let response = egui::Modal::new(Id::new("render-queue-sheet"))
        .backdrop_color(p.scrim)
        .frame(sheets::sheet(p))
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect();
            ui.set_width(560.0f32.min(screen.width() - 64.0).max(240.0));
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Render queue")
                        .font(theme::semibold(15.0))
                        .color(p.fg),
                );
                let summary = summary(jobs, waiting);
                if !summary.is_empty() {
                    ui.label(
                        RichText::new(summary)
                            .font(theme::regular(12.0))
                            .color(p.muted),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, p, Icon::Close, "Close render queue").clicked() {
                        answer = Some(QueueAnswer::Close);
                    }
                });
            });

            if jobs.is_empty() {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 170.0), Sense::hover());
                widgets::empty_state(
                    ui,
                    p,
                    rect,
                    Icon::Export,
                    "Nothing to render",
                    "Exports wait here and render one at a time in the background while \
                     you keep editing.",
                );
                ui.vertical_centered(|ui| {
                    if widgets::button(ui, p, Some(Icon::Export), "New export…", Kind::Primary)
                        .clicked()
                    {
                        answer = Some(QueueAnswer::NewExport);
                    }
                });
                ui.add_space(8.0);
                return;
            }

            let room = (screen.height() - 190.0).clamp(140.0, 520.0);
            // The sheet is centred by its height, and the list is as tall as
            // its rows up to the room there is. Their height is known only
            // once they are laid out: it is kept from the frame before, and a
            // list whose height changed is laid out again before it is shown,
            // so the sheet does not creep into place over several frames.
            let known_id = Id::new("render-queue-height");
            let known: Option<f32> = ui.data(|data| data.get_temp(known_id));
            let height = known.unwrap_or(ROW_ESTIMATE * jobs.len() as f32).min(room);
            let order_id = Id::new("render-queue-order");
            let mut order: Order = ui.data_mut(|data| data.get_temp(order_id).unwrap_or_default());
            let rows = order.listed(jobs, waiting);
            // A job that has just started is scrolled into sight once.
            let under_way = rows
                .first()
                .filter(|job| under_way(job, waiting))
                .map(|job| job.id);
            let reveal = under_way.filter(|id| order.shown_under_way != Some(*id));
            let list = egui::ScrollArea::vertical()
                .id_salt("render-queue-jobs")
                .max_height(room)
                .min_scrolled_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for job in &rows {
                        let place = waiting
                            .iter()
                            .position(|id| *id == job.id)
                            .map(|at| (at, waiting.len()));
                        let row = ui.scope(|ui| job_row(ui, p, job, place, home.as_deref()));
                        if let Some(asked) = row.inner {
                            answer = Some(asked);
                        }
                        if reveal == Some(job.id) {
                            ui.scroll_to_rect(row.response.rect, Some(Align::Min));
                        }
                    }
                });
            order.shown_under_way = under_way;
            ui.data_mut(|data| data.insert_temp(order_id, order));
            let content = list.content_size.y;
            if known.is_none_or(|known| (known - content).abs() > 0.5) {
                ui.data_mut(|data| data.insert_temp(known_id, content));
                ui.ctx()
                    .request_discard("the render queue's rows changed height");
            }

            ui.horizontal(|ui| {
                let finished = jobs.iter().any(|job| job.state.is_finished());
                let clear = ui
                    .add_enabled_ui(finished, |ui| {
                        widgets::button(ui, p, None, "Clear finished", Kind::Plain)
                    })
                    .inner
                    .on_hover_text("Take finished, failed and cancelled jobs off the list")
                    .on_disabled_hover_text("No job has finished");
                if clear.clicked() && finished {
                    answer = Some(QueueAnswer::ClearFinished);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button(ui, p, Some(Icon::Export), "New export…", Kind::Plain)
                        .clicked()
                    {
                        answer = Some(QueueAnswer::NewExport);
                    }
                    // Held jobs start only when asked: all of them at once.
                    let held = (jobs.iter()).filter(|job| is_held(job, waiting)).count();
                    if held > 0
                        && widgets::button(ui, p, Some(Icon::Play), "Start all", Kind::Plain)
                            .on_hover_text(match held {
                                1 => "Start the held job".to_owned(),
                                n => format!("Start the {n} held jobs, in the order listed"),
                            })
                            .clicked()
                    {
                        answer = Some(QueueAnswer::StartAll);
                    }
                });
            });
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

/// The state as a word, its ink and its fill.
fn badge_of(p: &Palette, job: &RenderJobInfo, held: bool) -> (String, Color32, Color32) {
    let tinted = |color: Color32| (color, theme::tint(color, 0.16));
    let (text, (ink, fill)) = match job.state {
        RenderState::Paused if held => ("Held".to_owned(), tinted(p.yellow)),
        RenderState::Queued => ("Queued".to_owned(), (p.secondary, p.control)),
        RenderState::Preparing => ("Preparing".to_owned(), tinted(p.accent)),
        RenderState::Rendering => (
            format!("{:.0} %", (job.progress.clamp(0.0, 1.0) * 100.0).floor()),
            tinted(p.accent),
        ),
        RenderState::Paused => ("Paused".to_owned(), tinted(p.yellow)),
        RenderState::Finalizing => ("Finishing".to_owned(), tinted(p.accent)),
        RenderState::Done => ("Done".to_owned(), tinted(p.green)),
        RenderState::Failed => ("Failed".to_owned(), tinted(p.red)),
        RenderState::Cancelled => ("Cancelled".to_owned(), (p.muted, p.control)),
    };
    (text, ink, fill)
}

/// The state, as a small pill.
fn badge(ui: &mut Ui, p: &Palette, job: &RenderJobInfo, held: bool) {
    let (text, ink, fill) = badge_of(p, job, held);
    let galley = ui
        .painter()
        .layout_no_wrap(text.clone(), theme::medium(11.0), ink);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 14.0, 20.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        ui.painter().galley(
            pos2(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            ink,
        );
    }
}

/// One job: its name, where it goes, its state and progress, and the
/// actions its state allows. `place` is its place among waiting jobs, held
/// ones included, and how many wait; a paused job with a place is held.
fn job_row(
    ui: &mut Ui,
    p: &Palette,
    job: &RenderJobInfo,
    place: Option<(usize, usize)>,
    home: Option<&Path>,
) -> Option<QueueAnswer> {
    let mut answer = None;
    let name = job.name.as_str();
    let held = job.state == RenderState::Paused && place.is_some();
    sheets::group(ui, p, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.set_min_height(metrics::CONTROL_HEIGHT);
            let (glyph, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            let (icon, tint) = match job.state {
                RenderState::Done => (Icon::Check, p.green),
                RenderState::Failed => (Icon::Warning, p.red),
                _ if job.settings.video.is_none() => (Icon::Music, p.secondary),
                _ => (Icon::Film, p.secondary),
            };
            icons::paint(ui.painter(), glyph, icon, tint);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // Trailing first: the actions, then the state.
                let mut act = |ui: &mut Ui, icon, label: String, enabled: bool, asked| {
                    let clicked = ui
                        .add_enabled_ui(enabled, |ui| icon_button(ui, p, icon, &label))
                        .inner
                        .clicked();
                    if clicked && enabled {
                        answer = Some(asked);
                    }
                };
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
                    RenderState::Queued | RenderState::Paused
                        if held || job.state == RenderState::Queued =>
                    {
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
                ui.add_space(4.0);
                badge(ui, p, job, held);
                ui.add_space(6.0);
                let room = ui.available_width();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let galley = widgets::truncated(ui, name, theme::medium(12.5), p.fg, room);
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(room.max(0.0), metrics::CONTROL_HEIGHT),
                        Sense::hover(),
                    );
                    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
                    ui.painter().galley(
                        pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
                        galley,
                        p.fg,
                    );
                    if job.name.len() > 24 {
                        response.on_hover_text(&job.name);
                    }
                });
            });
        });

        // Where it goes, under the name: the last folders, which tell jobs
        // apart, with the whole path on hover.
        let folder = (job.destination.parent())
            .filter(|folder| !folder.as_os_str().is_empty())
            .map(|folder| short_folder(folder, home));
        if let Some(folder) = folder {
            let width = ui.available_width() - 22.0;
            ui.horizontal(|ui| {
                // The folder glyph under the job's, the folder under its name,
                // where the lines below begin.
                ui.spacing_mut().item_spacing.x = 0.0;
                let (gutter, _) = ui.allocate_exact_size(vec2(22.0, 15.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(gutter.left() + 8.0, gutter.center().y),
                        Vec2::splat(12.0),
                    ),
                    Icon::Folder,
                    p.muted,
                );
                let galley = widgets::truncated(ui, &folder, theme::regular(11.5), p.muted, width);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(width.max(0.0), 15.0), Sense::hover());
                response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Label,
                        true,
                        format!("Saved in {}", job.destination.display()),
                    )
                });
                ui.painter().galley(rect.left_top(), galley, p.muted);
                response.on_hover_text(job.destination.display().to_string());
            });
        }

        // A held job has not started: it has no progress to show.
        let moving = !held
            && matches!(
                job.state,
                RenderState::Preparing
                    | RenderState::Rendering
                    | RenderState::Paused
                    | RenderState::Finalizing
            );
        if moving {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.vertical(|ui| {
                    ui.add_space(3.0);
                    let fill = if job.state == RenderState::Paused {
                        p.secondary
                    } else {
                        p.accent
                    };
                    widgets::progress(ui, p, job.progress, fill, &format!("Progress of “{name}”"));
                    ui.add_space(1.0);
                });
            });
        }

        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ui.label(
                RichText::new(status(job, held))
                    .font(theme::regular(11.5))
                    .color(p.secondary),
            );
            if let Some(encoder) = &job.encoder {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(encoder)
                            .font(theme::mono(10.5))
                            .color(p.muted),
                    );
                });
            }
        });
        if let Some(message) = job.message.as_deref().filter(|m| !m.trim().is_empty()) {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                let color = if job.state == RenderState::Failed {
                    p.red
                } else {
                    p.secondary
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(message)
                            .font(theme::regular(11.5))
                            .color(color),
                    )
                    .wrap(),
                );
            });
        }
    });
    answer
}

/// A small ring for the toolbar's queue button: the track, the share of the
/// work done as an arc from the top, and the number of jobs under way when
/// more than one is. Painted only; the button around it carries the name.
pub fn summary_ring(ui: &Ui, p: &Palette, rect: Rect, progress: f32, active: usize) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.5 - 1.5;
    painter.circle_stroke(center, radius, Stroke::new(2.0, p.pressed));
    let done = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if done > 0.0 {
        let steps = ((done * 48.0).ceil() as usize).max(2);
        let points: Vec<_> = (0..=steps)
            .map(|step| {
                let angle = -TAU / 4.0 + TAU * done * step as f32 / steps as f32;
                center + radius * Vec2::angled(angle)
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0, p.accent)));
    }
    if active > 1 {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            active.min(99).to_string(),
            theme::medium((radius * 1.1).clamp(7.0, 11.0)),
            p.fg,
        );
    }
}

#[cfg(test)]
#[path = "render_queue_tests.rs"]
mod tests;
