//! Sheets and messages that sit over the window: Preferences, the questions
//! asked before work is replaced or given up, toasts in the window's corner,
//! the cards of work under way over the viewer, and the target that files are
//! dropped on.
//!
//! Every sheet is Terminal's: one elevated surface over a rounded scrim, its
//! title at the top and its answers at the trailing edge of its foot, the one
//! that confirms furthest out. Escape gives each sheet's own answer, as it
//! always has; Enter gives the default only when it is pressed after the sheet
//! is in sight, and never gives up work.
use super::{
    Action, PreferencesPane,
    controls::{self, ButtonKind, ButtonSize, Group},
    kit, shortcut,
    widgets::Kind,
};
use crate::{
    ai::{Effort, Provider},
    extract::{Card, Shown},
    icons::{self, Icon},
    proxies::Cache,
    settings::{PreviewQuality, PreviewSettings, ProxyMode, Scale, Settings},
    theme::{self, Accent, Appearance, Palette, metrics, motion},
};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Event, Galley, Id, Key, Layout, Pos2, Rect, Sense,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use std::{cell::Cell, path::PathBuf, sync::Arc};

/// What waits for an answer about unsaved work.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    New,
    /// Open this project, or ask which when there is no path.
    Open(Option<PathBuf>),
    Quit,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Overlay {
    #[default]
    None,
    Settings,
    Checkpoints,
    Unsaved(Pending),
    /// A save was refused because another writer has the file.
    Conflict(Conflict),
    /// The export sheet; its state is the application's while it is open.
    Export,
    /// An export would replace this file, which is there: asked over the
    /// export sheet, which keeps its state.
    ReplaceExport(String),
    /// The render queue.
    RenderQueue,
    /// Quitting would stop exports that are rendering or waiting.
    StopRendering,
    /// A duck would replace volume keyframes the clips have.
    Duck(DuckQuestion),
}

/// A duck that waits to be told it may replace the volume keyframes of
/// `keyed` of its clips. Nothing has been read or written yet.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckQuestion {
    pub clips: Vec<editor_core::ClipId>,
    pub depth: f32,
    pub keyed: usize,
}

/// A save that was not made, so that another writer's work is not replaced.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub path: PathBuf,
    /// The other writer was only in the middle of its own save.
    pub busy: bool,
    /// The file is the one this window has open, so it can be reopened.
    pub open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictAnswer {
    SaveAs,
    /// Give up the edits here and read the file as it now is.
    Reopen,
    /// Ask for the same save again.
    Retry,
    Later,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

/// What the person said to the recovery copy a window found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    Restore,
    /// The copy is given up and removed.
    Discard,
    /// Nothing is decided: the copy stays for next time.
    Later,
}

/// A named copy of the project, kept beside its file.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    pub path: PathBuf,
    pub label: String,
    /// Seconds since the Unix epoch at which it was made, when known.
    pub made: Option<u64>,
}

#[derive(Default)]
pub struct CheckpointsState {
    pub name: String,
    /// `None` until the worker has read them.
    pub list: Option<Result<Vec<Checkpoint>, String>>,
    /// A checkpoint is being written, read or listed.
    pub busy: bool,
    pub focus: bool,
}

pub enum CheckpointAnswer {
    Close,
    Create(String),
    Restore(Checkpoint),
    /// The project has no file to keep checkpoints beside yet.
    SaveProject,
}

pub struct Notice {
    pub text: String,
    pub problem: bool,
    /// Seconds since the application started at which it goes away: never,
    /// for a problem, which stays until it is dismissed.
    pub until: f64,
}

impl Notice {
    /// Who the notice is to its Dismiss and to its clock: its words, which
    /// no two notices share.
    pub fn id(&self) -> u64 {
        Id::new(&self.text).value()
    }
}

/// Lays `add` out at the cursor in a column exactly `width` wide, for what
/// a sheet scrolls.
///
/// A scroll area rounds its content's room to device pixels where it is on
/// the screen, so what fills that room can end a fraction of a pixel past
/// the area. A sheet is centred on the width it had the frame before: that
/// fraction moves it, the move turns the rounding the other way, and at a
/// fractional scale the whole sheet shook by a pixel at every repaint. What
/// is in the column cannot widen the sheet.
pub(crate) fn column<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let room = Rect::from_min_size(ui.cursor().min, vec2(width, ui.available_height()));
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(room)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let inner = add(&mut column);
    ui.advance_cursor_after_rect(Rect::from_min_size(
        room.min,
        vec2(width, column.min_rect().height()),
    ));
    inner
}

// ---- The scaffold ----------------------------------------------------------

/// The sheets asked over another sheet, each with the one it is asked over.
/// That one already dims the window: one scrim is enough, so they bring
/// none of their own. It is still open, and toasts keep clear of both.
const ASKED_OVER_A_SHEET: [(&str, &str); 1] = [("replace-export-sheet", "export-sheet")];

/// Dims the window under the sheet `id`, in the window's own rounded shape,
/// painted first into the sheet's own layer: under the sheet, and over
/// everything the sheet covers, the cards over the viewer included. It
/// fades in as the sheet first shows, at once under Reduce motion.
fn scrim(ctx: &egui::Context, p: &Palette, id: Id) {
    let key = id.with("scrim");
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|input| input.time);
    let since = ctx.data_mut(|data| {
        // Shown in this pass or the one before: the same showing.
        let since = (data.get_temp::<(u64, f64)>(key))
            .filter(|(shown, _)| shown + 1 >= pass)
            .map_or(now, |(_, since)| since);
        data.insert_temp(key, (pass, since));
        since
    });
    let dim = if kit::reduced_motion(ctx) {
        1.0
    } else {
        let t = ((now - since) as f32 / motion::SHEET).clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint();
        }
        egui::emath::easing::cubic_out(t)
    };
    controls::scrim(
        &ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, id)),
        ctx.content_rect(),
        super::window::corner_radius(ctx),
        *p,
        dim,
    );
}

/// Whether the sheet `id` is shown in this frame for the first time since it
/// was last away: the frame whose keys were meant for what was there before.
/// Asked again in the same frame, it says the same.
fn opening(ctx: &egui::Context, id: Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let key = id.with("shown");
    let seen = ctx.data(|data| data.get_temp::<(u64, u64)>(key));
    let first = match seen {
        Some((first, last)) if last == frame || last + 1 == frame => first,
        _ => frame,
    };
    ctx.data_mut(|data| data.insert_temp(key, (first, frame)));
    first == frame
}

/// What a sheet heard in one frame besides its own controls.
struct Heard<R> {
    inner: R,
    /// Escape, or a click beside the sheet: each sheet answers them as it
    /// always has. An Escape that only took the keyboard out of a field, or
    /// that the sheet took for itself, is not one.
    dismissed: bool,
    /// A fresh Enter while nothing has the keyboard: the default answer.
    enter: bool,
    /// An Enter carried over from what was there before the sheet, or held
    /// down: it answers nothing, not even on a button that has the keyboard.
    carried: bool,
}

/// How tall a sheet that asks for `requested` is in a window this tall.
/// Centred, it keeps clear of the title bar, as Preferences does, and is
/// short only where the window is: in a window under about 600 tall it
/// gives up height before it covers the toolbar, down to 240.
fn sheet_height(requested: f32, window: f32) -> f32 {
    let clear = window - 2.0 * (metrics::TOOLBAR_HEIGHT + 4.0);
    requested
        .min(clear)
        .max(requested.min(240.0).min(window - 24.0))
}

/// Terminal's sheet under its kept id: elevated, rounded, outlined and
/// shadowed, `width` wide or the window less 12 on each side, with the
/// window's rounded scrim under it rather than egui's square one. Assistive tools
/// know it as a window named `title`. `escape_taken` says the sheet used
/// this frame's Escape itself.
fn scaffold<R>(
    ctx: &egui::Context,
    p: &Palette,
    id: &str,
    title: &str,
    width: f32,
    escape_taken: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Heard<R> {
    let name = id;
    let id = Id::new(id);
    let opening = opening(ctx, id);
    let typing_id = id.with("typing");
    // Escape takes the keyboard out of a field before egui shows the frame,
    // so whether one had it is what the last frame said.
    let was_typing = ctx.data(|data| data.get_temp::<bool>(typing_id)) == Some(true);
    let focused = ctx.memory(|memory| memory.focused()).is_some();
    let (fresh, carried) = ctx.input(|input| {
        input
            .events
            .iter()
            .fold((false, false), |(fresh, carried), event| match event {
                Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    repeat,
                    ..
                } => {
                    let old = *repeat || opening;
                    (fresh || !old, carried || old)
                }
                _ => (fresh, carried),
            })
    });
    let screen = ctx.content_rect();
    let frame = egui::Frame::new()
        .fill(p.elevated)
        .corner_radius(metrics::SHEET_RADIUS)
        .stroke(Stroke::new(1.0, p.border))
        .shadow(p.sheet_shadow());
    if !ASKED_OVER_A_SHEET.iter().any(|(asked, _)| *asked == name) {
        scrim(ctx, p, id);
    }
    let modal = egui::Modal::new(id)
        .frame(frame)
        // The rounded scrim stands in for egui's square one.
        .backdrop_color(Color32::TRANSPARENT)
        .show(ctx, |ui| {
            ui.set_width(width.min(screen.width() - 24.0).max(120.0));
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            add(ui)
        });
    modal
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Window, true, title));
    let typing = ctx
        .memory(|memory| memory.focused())
        .is_some_and(|focused| egui::text_edit::TextEditState::load(ctx, focused).is_some());
    ctx.data_mut(|data| data.insert_temp(typing_id, typing));
    let escape = ctx.input(|input| input.key_pressed(Key::Escape));
    let dismissed = if escape && (was_typing || escape_taken) {
        modal.backdrop_response.clicked()
    } else {
        modal.should_close()
    };
    Heard {
        enter: fresh && !carried && !focused && modal.is_top_modal && !modal.any_popup_open,
        inner: modal.inner,
        dismissed,
        carried,
    }
}

/// A sheet's 52-point title row, with a spinner beside the title while the
/// sheet waits on work. Returns whether its close control was used.
fn header(ui: &mut Ui, p: &Palette, title: &str, close: Option<&str>, busy: bool) -> bool {
    let top = ui.cursor().top();
    let left = ui.max_rect().left();
    let closed = controls::sheet_header(ui, *p, title, close);
    title_said(ui, pos2(left, top), title);
    if busy {
        let width = (ui.painter())
            .layout_no_wrap(title.to_owned(), theme::semibold(15.0), p.fg)
            .size()
            .x;
        kit::spinner(
            ui,
            Rect::from_center_size(
                pos2(left + 20.0 + width + 16.0, top + 26.0),
                Vec2::splat(14.0),
            ),
            p.muted,
        );
    }
    closed
}

/// Tells assistive tools of the title `controls::sheet_header` painted in
/// the row whose top leading corner is `at`.
fn title_said(ui: &Ui, at: Pos2, title: &str) {
    if !title.is_empty() {
        let words = painted(
            ui,
            pos2(at.x + 20.0, at.y + 26.0),
            title,
            theme::semibold(15.0),
        );
        said(ui, words, title);
    }
}

/// One answer a dialog offers: its words, its kind and what it says. The
/// kind is the one the areas name buttons by.
type Offer<'a, A> = (&'a str, Kind, A);

/// The answer a dialog offers apart, at its leading edge, which gives
/// something up, with what it gives up said when the pointer rests on it.
type Leading<'a, A> = (Offer<'a, A>, Option<&'a str>);

/// The ported control's kind of a button the areas name: a Plain one is
/// Terminal's Secondary.
fn kind(kind: Kind) -> ButtonKind {
    match kind {
        Kind::Plain => ButtonKind::Secondary,
        Kind::Primary => ButtonKind::Primary,
        Kind::Destructive => ButtonKind::Destructive,
        Kind::Quiet => ButtonKind::Quiet,
    }
}

/// Terminal's dialog foot, 62 tall: the answers at the trailing edge 8
/// apart and 20 in, the confirming one furthest out, and one that gives
/// something up alone at the leading edge. Hands back the answer clicked.
fn footer<A: Copy>(
    ui: &mut Ui,
    p: &Palette,
    leading: Option<Leading<A>>,
    trailing: &[Offer<A>],
) -> Option<A> {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 62.0));
    let room = rect.shrink2(vec2(20.0, 0.0));
    let mut answer = None;
    controls::place(
        ui,
        room,
        Layout::right_to_left(Align::Center),
        "dialog-actions",
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for &(label, of, said) in trailing {
                if controls::button(ui, *p, label, kind(of)).clicked() {
                    answer = Some(said);
                }
            }
        },
    );
    if let Some(((label, of, said), hint)) = leading {
        controls::place(
            ui,
            room,
            Layout::left_to_right(Align::Center),
            "dialog-leading",
            |ui| {
                let button = controls::button(ui, *p, label, kind(of));
                let button = match hint {
                    Some(hint) => button.on_hover_text(hint),
                    None => button,
                };
                if button.clicked() {
                    answer = Some(said);
                }
            },
        );
    }
    answer
}

/// The default of a dialog: its Primary, never what gives work up. A dialog
/// without one has no default, and Enter does nothing in it.
fn default_of<A: Copy>(trailing: &[Offer<A>]) -> Option<A> {
    (trailing.iter())
        .find(|(_, kind, _)| *kind == Kind::Primary)
        .map(|(.., said)| *said)
}

/// A dialog's title, wrapped.
fn title(ui: &mut Ui, p: &Palette, text: &str, size: f32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::semibold(size))
                .color(p.fg),
        )
        .wrap()
        .selectable(false),
    );
}

/// A dialog's message under its title, wrapped.
fn message(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(13.0))
                .color(p.secondary),
        )
        .wrap()
        .selectable(false),
    );
}

/// The mark that leads a sheet asking about work that could be lost.
fn warning(ui: &mut Ui, p: &Palette) {
    let (_, glyph) = ui.allocate_space(Vec2::splat(32.0));
    icons::paint(ui.painter(), glyph, Icon::Warning, p.legible(p.attention));
    ui.add_space(12.0);
}

/// Terminal's confirmation: 22 points of air, then the words `body` lays
/// out 22 in, then the dialog foot. Escape says `escape`; a fresh Enter
/// says the Primary, where there is one.
#[expect(
    clippy::too_many_arguments,
    reason = "each dialog names its own words and answers; a struct would only rename them"
)]
fn dialog<A: Copy>(
    ctx: &egui::Context,
    p: &Palette,
    id: &str,
    width: f32,
    name: &str,
    body: impl FnOnce(&mut Ui),
    leading: Option<Leading<A>>,
    trailing: &[Offer<A>],
    escape: A,
) -> Option<A> {
    let heard = scaffold(ctx, p, id, name, width, false, |ui| {
        // Terminal's dialog: 22 over the body and at its sides (§5.9.5).
        ui.add_space(22.0);
        controls::padded(ui, 22.0, body);
        ui.add_space(6.0);
        footer(ui, p, leading, trailing)
    });
    if heard.carried {
        // The Enter that opened the dialog is not an answer to it.
        return None;
    }
    heard
        .inner
        .or_else(|| heard.enter.then(|| default_of(trailing)).flatten())
        .or_else(|| heard.dismissed.then_some(escape))
}

// ---- Confirmations ---------------------------------------------------------

/// Asks what to do with unsaved work before it is replaced.
pub fn unsaved(ctx: &egui::Context, p: &Palette, name: &str, pending: &Pending) -> Option<Answer> {
    let question = format!("Save changes to “{name}”?");
    dialog(
        ctx,
        p,
        "unsaved-sheet",
        360.0,
        &question,
        |ui| {
            title(ui, p, &question, 15.0);
            message(
                ui,
                p,
                match pending {
                    Pending::Quit => "Your changes are lost if you quit without saving.",
                    _ => "Your changes are lost if you continue without saving.",
                },
            );
        },
        Some((("Don't save", Kind::Destructive, Answer::Discard), None)),
        &[
            ("Save", Kind::Primary, Answer::Save),
            ("Cancel", Kind::Plain, Answer::Cancel),
        ],
        Answer::Cancel,
    )
}

/// Asks before an export replaces `file`, which is in `folder`, and `also`,
/// its caption file, when that is there too. `true` replaces them; Cancel,
/// Escape and the backdrop answer `false`.
pub fn replace_export(
    ctx: &egui::Context,
    p: &Palette,
    file: &str,
    also: Option<&str>,
    folder: &str,
) -> Option<bool> {
    let (question, body) = match also {
        None => (
            format!("Replace “{file}”?"),
            format!(
                "A file with this name is already in “{folder}”. It is replaced once the \
                 export has finished; until then it stays as it is."
            ),
        ),
        Some(also) => (
            format!("Replace “{file}” and “{also}”?"),
            format!(
                "Files with these names, the export and its caption file, are already in \
                 “{folder}”. They are replaced once the export has finished; until then \
                 they stay as they are."
            ),
        ),
    };
    dialog(
        ctx,
        p,
        "replace-export-sheet",
        420.0,
        &question,
        |ui| {
            title(ui, p, &question, 15.0);
            message(ui, p, &body);
        },
        None,
        &[
            ("Replace", Kind::Destructive, true),
            ("Cancel", Kind::Plain, false),
        ],
        false,
    )
}

/// Asks before quitting stops the `jobs` exports rendering or waiting.
/// `true` cancels them at once and goes on to quit; Keep rendering, Escape
/// and the backdrop answer `false`.
pub fn stop_rendering(ctx: &egui::Context, p: &Palette, jobs: usize) -> Option<bool> {
    let body = match jobs {
        1 => "An export is rendering or waiting. Stop and quit cancels it now and \
              removes its unfinished file; a file it would replace is left as it is. \
              If you then stay, it can be rendered again from the render queue."
            .to_owned(),
        n => format!(
            "{n} exports are rendering or waiting. Stop and quit cancels them now and \
             removes their unfinished files; files they would replace are left as they \
             are. If you then stay, they can be rendered again from the render queue."
        ),
    };
    let question = "Rendering will stop";
    dialog(
        ctx,
        p,
        "stop-rendering-sheet",
        380.0,
        question,
        |ui| {
            title(ui, p, question, 15.0);
            message(ui, p, &body);
        },
        None,
        &[
            ("Stop and quit", Kind::Destructive, true),
            ("Keep rendering", Kind::Plain, false),
        ],
        false,
    )
}

/// Asks whether a duck may replace the volume keyframes of the clips that
/// have them. `Some(true)` to go on; `Some(false)` when it is given up,
/// which an Escape or a click outside the sheet also says.
pub fn replace_keys(ctx: &egui::Context, p: &Palette, keyed: usize) -> Option<bool> {
    let question = crate::duck::question(keyed);
    dialog(
        ctx,
        p,
        "duck-sheet",
        420.0,
        &question,
        |ui| {
            title(ui, p, &question, 15.0);
            message(
                ui,
                p,
                "Ducking writes new volume keyframes in their place. Undo brings them back.",
            );
        },
        None,
        &[
            ("Replace", Kind::Destructive, true),
            ("Cancel", Kind::Plain, false),
        ],
        false,
    )
}

/// Says that a save was refused for another writer's sake, and asks what to
/// do with the edits, which are still in the window. Nothing here writes the
/// file over what the other writer left, and nothing gives the edits up
/// unless it is said in so many words.
pub fn conflict(
    ctx: &egui::Context,
    p: &Palette,
    name: &str,
    conflict: &Conflict,
) -> Option<ConflictAnswer> {
    let file = conflict.path.file_name().map_or_else(
        || conflict.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let question = if conflict.busy {
        format!("“{file}” is being saved elsewhere")
    } else {
        format!("“{file}” changed on disk")
    };
    let body = if conflict.busy {
        format!(
            "Another editor is writing this file right now, so “{name}” was not saved. \
             Your edits are still here. Try again in a moment, or save them to another \
             file."
        )
    } else if conflict.open {
        format!(
            "Something else saved this file after it was opened here, so “{name}” was \
             not saved over it. Your edits are still here. Save them to another file, \
             or give them up and reopen the file as it is now."
        )
    } else {
        format!(
            "Something else wrote this file after it was chosen, so “{name}” was not \
             saved over it. Your edits are still here. Save them to another file."
        )
    };
    let path = conflict.path.display().to_string();
    let trailing: &[Offer<ConflictAnswer>] = if conflict.busy {
        &[
            ("Try again", Kind::Primary, ConflictAnswer::Retry),
            ("Save as…", Kind::Plain, ConflictAnswer::SaveAs),
            ("Not now", Kind::Plain, ConflictAnswer::Later),
        ]
    } else {
        &[
            ("Save as…", Kind::Primary, ConflictAnswer::SaveAs),
            ("Not now", Kind::Plain, ConflictAnswer::Later),
        ]
    };
    let reopen = (conflict.open && !conflict.busy).then_some((
        (
            "Discard and reopen",
            Kind::Destructive,
            ConflictAnswer::Reopen,
        ),
        Some("Give up the edits in this window and read the file again"),
    ));
    dialog(
        ctx,
        p,
        "conflict-sheet",
        400.0,
        &question,
        |ui| {
            warning(ui, p);
            title(ui, p, &question, 15.0);
            message(ui, p, &body);
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(path)
                        .font(theme::mono(11.0))
                        .color(p.secondary),
                )
                .wrap()
                .selectable(false),
            );
        },
        reopen,
        trailing,
        ConflictAnswer::Later,
    )
}

/// Offers the recovery copy a window found of unsaved work: of the project
/// it opened when `project`, or of one that was never saved. Restore, which
/// keeps the work, is the default; Discard leads, apart. It has no close
/// control, since it needs an answer, and Escape decides nothing.
pub fn recovery(ctx: &egui::Context, p: &Palette, project: bool) -> Option<Recovery> {
    let question = "Restore unsaved changes?";
    dialog(
        ctx,
        p,
        "recovery-sheet",
        520.0,
        question,
        |ui| {
            warning(ui, p);
            title(ui, p, question, 17.0);
            message(
                ui,
                p,
                if project {
                    "This project was closed with changes that were not saved. A recovery \
                     copy of them was kept."
                } else {
                    "A project that was never saved was open when Neptune Video last closed. \
                     A recovery copy of it was kept."
                },
            );
        },
        Some((("Discard", Kind::Destructive, Recovery::Discard), None)),
        &[("Restore", Kind::Primary, Recovery::Restore)],
        Recovery::Later,
    )
}

// ---- Checkpoints -----------------------------------------------------------

/// Named checkpoints of the project: make one, or go back to one.
pub fn checkpoints(
    ctx: &egui::Context,
    p: &Palette,
    state: &mut CheckpointsState,
    has_file: bool,
) -> Option<CheckpointAnswer> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let screen = ctx.content_rect();
    let height = sheet_height(520.0, screen.height());
    let mut create = false;
    let heard = scaffold(
        ctx,
        p,
        "checkpoints-sheet",
        "Checkpoints",
        520.0,
        false,
        |ui| {
            let top = ui.cursor().top();
            let mut answer = None;
            if header(ui, p, "Checkpoints", Some("Close checkpoints"), state.busy) {
                answer = Some(CheckpointAnswer::Close);
            }
            if !has_file {
                controls::padded(ui, 20.0, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Checkpoints are kept beside the project file.")
                                .font(theme::regular(13.0))
                                .color(p.secondary),
                        )
                        .wrap()
                        .selectable(false),
                    );
                });
                ui.add_space(6.0);
                let save = footer(ui, p, None, &[("Save project…", Kind::Primary, ())]);
                return answer.or(save.map(|()| CheckpointAnswer::SaveProject));
            }

            // The name of the next one, and the button that makes it.
            controls::padded(ui, 20.0, |ui| {
                let (_, row) =
                    ui.allocate_space(vec2(ui.available_width(), metrics::CONTROL_HEIGHT));
                let save = controls::place(
                    ui,
                    row,
                    Layout::right_to_left(Align::Center),
                    "checkpoint-save",
                    |ui| {
                        ui.add_enabled_ui(!state.busy, |ui| {
                            controls::button(ui, *p, "Save", ButtonKind::Primary)
                        })
                        .inner
                        .on_hover_text("Save a checkpoint of the project as it is now")
                    },
                );
                let field_rect =
                    Rect::from_min_max(row.min, pos2(save.rect.left() - 8.0, row.max.y));
                let field = controls::place(
                    ui,
                    field_rect,
                    Layout::left_to_right(Align::Center),
                    "checkpoint-field",
                    |ui| {
                        controls::text_field(
                            ui,
                            *p,
                            Id::new("checkpoint-name"),
                            &mut state.name,
                            "Checkpoint name",
                            "Checkpoint name",
                            field_rect.width(),
                        )
                    },
                );
                // A sheet measures itself in a hidden first pass, where the
                // keyboard cannot be given.
                if state.focus && !ui.is_sizing_pass() {
                    field.request_focus();
                    state.focus = false;
                }
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                create = save.clicked() || entered;
            });
            // The list's card stands 14 under the field, as Preferences'
            // cards stand apart (§5.9.2).
            ui.add_space(14.0);

            // The checkpoints, newest first as the worker lists them, in
            // the rest of a sheet of one height, so that saving one never
            // moves the field being typed in.
            let rest = (height - (ui.cursor().top() - top)).max(108.0);
            let (_, body) = ui.allocate_space(vec2(ui.available_width(), rest));
            controls::place(
                ui,
                body,
                Layout::top_down(Align::Min),
                "checkpoints-body",
                |ui| match &state.list {
                    None => {}
                    Some(Err(problem)) => controls::padded(ui, 20.0, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(problem)
                                    .font(theme::regular(13.0))
                                    .color(p.red),
                            )
                            .wrap(),
                        );
                    }),
                    Some(Ok(list)) if list.is_empty() => {
                        kit::empty_state(
                            ui,
                            p,
                            body.with_max_y(body.bottom() - 20.0),
                            Icon::History,
                            "No checkpoints yet",
                            "",
                            None,
                        );
                    }
                    Some(Ok(list)) => {
                        let scrolled = egui::ScrollArea::vertical()
                            .id_salt("checkpoints-list")
                            .max_height(body.height() - 20.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                controls::padded(ui, 20.0, |ui| {
                                    controls::group(ui, *p, |ui, rows| {
                                        for checkpoint in list {
                                            if checkpoint_row(
                                                ui,
                                                p,
                                                rows,
                                                checkpoint,
                                                now,
                                                !state.busy,
                                            ) {
                                                answer = Some(CheckpointAnswer::Restore(
                                                    checkpoint.clone(),
                                                ));
                                            }
                                        }
                                    });
                                });
                            });
                        fade_below(ui, p, &scrolled);
                    }
                },
            );
            answer
        },
    );
    if heard.carried {
        // The Enter that opened the sheet is not an answer to it.
        return heard.dismissed.then_some(CheckpointAnswer::Close);
    }
    // Enter with nothing holding the keyboard is the Primary's.
    if !has_file && heard.enter {
        return Some(CheckpointAnswer::SaveProject);
    }
    if (create || heard.enter) && has_file && !state.busy {
        let name = state.name.trim();
        let name = if name.is_empty() {
            "Checkpoint".to_owned()
        } else {
            name.to_owned()
        };
        state.name.clear();
        return Some(CheckpointAnswer::Create(name));
    }
    heard
        .inner
        .or_else(|| heard.dismissed.then_some(CheckpointAnswer::Close))
}

/// One checkpoint, 44 tall: its name, its age and the way back to it, which
/// is always there for the keyboard to reach.
fn checkpoint_row(
    ui: &mut Ui,
    p: &Palette,
    rows: &mut Group,
    checkpoint: &Checkpoint,
    now: u64,
    ready: bool,
) -> bool {
    let rect = rows.custom(ui, 44.0);
    let restore = controls::place(
        ui,
        rect.shrink2(vec2(12.0, 0.0)),
        Layout::right_to_left(Align::Center),
        ("checkpoint", &checkpoint.path),
        |ui| {
            let restore = ui
                .add_enabled_ui(ready, |ui| {
                    controls::button_sized(
                        ui,
                        *p,
                        "Restore",
                        ButtonKind::Secondary,
                        ButtonSize::Compact,
                    )
                })
                .inner;
            restore.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::Button,
                    ready,
                    format!("Restore {}", checkpoint.label),
                )
            });
            restore
        },
    );
    let painter = ui.painter().clone();
    let mut right = restore.rect.left() - 12.0;
    let mut age_at = None;
    if let Some(made) = checkpoint.made {
        let words = crate::timefmt::age(now.saturating_sub(made));
        let age = painter.layout_no_wrap(words.clone(), theme::mono(12.0), p.secondary);
        right -= age.size().x;
        let at = pos2(right, rect.center().y);
        age_at = Some((painted(ui, at, &words, theme::mono(12.0)), words));
        controls::galley_at(&painter, at, age);
        right -= 12.0;
    }
    let left = rect.left() + 14.0;
    let name = controls::elided(
        &painter,
        &checkpoint.label,
        theme::medium(13.0),
        p.fg,
        right - left,
    );
    let name_at = Rect::from_min_size(
        pos2(left, rect.center().y - name.size().y * 0.5),
        name.size(),
    );
    controls::galley_at(&painter, pos2(left, rect.center().y), name);
    // Its name and its age are said as they read, before the way back.
    said(ui, name_at, &checkpoint.label);
    if let Some((at, words)) = age_at {
        said(ui, at, &words);
    }
    restore.clicked()
}

/// Says that a sheet's list runs on below where it is cut off.
fn fade_below<R>(ui: &Ui, p: &Palette, scrolled: &egui::scroll_area::ScrollAreaOutput<R>) {
    let shown = scrolled.inner_rect;
    if scrolled.state.offset.y + shown.height() < scrolled.content_size.y - 1.0 {
        kit::scroll_fade(ui.painter(), shown, kit::Edge::Bottom, p.elevated);
    }
}

// ---- Preferences -----------------------------------------------------------

/// Preferences is this large when the window has room for it.
const PREFERENCES_WIDTH: f32 = 680.0;
const PREFERENCES_HEIGHT: f32 = 520.0;
const HEADER: f32 = 52.0;
/// The source list sits inside the sheet as a surface of its own.
const INSET: f32 = 6.0;
const SIDEBAR: f32 = 176.0;
/// A sheet too narrow for names keeps the source list as icons.
const RAIL: f32 = 44.0;
const COMPACT_BELOW: f32 = 540.0;

/// The glyph of a pane in the source list.
fn pane_icon(pane: PreferencesPane) -> Icon {
    match pane {
        PreferencesPane::Appearance => Icon::Palette,
        PreferencesPane::Editing => Icon::Pencil,
        PreferencesPane::Preview => Icon::Film,
        PreferencesPane::Agent => Icon::Sparkles,
        PreferencesPane::Keyboard => Icon::Keyboard,
    }
}

/// One thing a search can find: a row, or the rows that belong together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Appearance,
    Accent,
    InterfaceSize,
    ReduceMotion,
    ResetLayout,
    Snapping,
    RecoveryCopy,
    Quality,
    Proxies,
    ProxyCache,
    Provider,
    Model,
    Effort,
    Edits,
    Folders,
    /// A row of the Keyboard pane, by its place in [`SHORTCUTS`].
    Shortcut(usize),
}

/// Each setting with its pane, its label and the other words people use for
/// it. A search reads the pane's name and the label as well as these.
const INDEX: [(Setting, PreferencesPane, &str, &str); 15] = [
    (
        Setting::Appearance,
        PreferencesPane::Appearance,
        "Appearance",
        "theme dark light system mode night day match follow automatic auto colors colours",
    ),
    (
        Setting::Accent,
        PreferencesPane::Appearance,
        "Accent",
        "color colour highlight tint blue indigo purple pink red orange yellow green graphite",
    ),
    (
        Setting::InterfaceSize,
        PreferencesPane::Appearance,
        "Interface size",
        "scale scaling zoom bigger smaller larger magnify text ui dpi percent display",
    ),
    (
        Setting::ReduceMotion,
        PreferencesPane::Appearance,
        "Reduce motion",
        "animation animations animate fade fades slide slides still accessibility",
    ),
    (
        Setting::ResetLayout,
        PreferencesPane::Appearance,
        "Reset layout",
        "layout panels panel library inspector timeline widths sizes default restore",
    ),
    (
        Setting::Snapping,
        PreferencesPane::Editing,
        "Snap while dragging",
        "snapping snap magnet align edges playhead markers clips",
    ),
    (
        Setting::RecoveryCopy,
        PreferencesPane::Editing,
        "Keep a recovery copy while editing",
        "autosave auto save backup crash recover recovery restore unsaved",
    ),
    (
        Setting::Quality,
        PreferencesPane::Preview,
        "Preview quality",
        "resolution playback full half quarter auto detail performance smooth viewer",
    ),
    (
        Setting::Proxies,
        PreferencesPane::Preview,
        "Proxies",
        "proxy smaller copies playback performance optimized media off always auto",
    ),
    (
        Setting::ProxyCache,
        PreferencesPane::Preview,
        "Cache size",
        "proxy proxies cache disk space storage gigabytes gb clear remove",
    ),
    (
        Setting::Provider,
        PreferencesPane::Agent,
        "Answered by",
        "provider claude codex ai assistant model command line",
    ),
    (
        Setting::Model,
        PreferencesPane::Agent,
        "Model",
        "ai claude codex name version default",
    ),
    (
        Setting::Effort,
        PreferencesPane::Agent,
        "Effort",
        "reasoning thinking default low medium high",
    ),
    (
        Setting::Edits,
        PreferencesPane::Agent,
        "Edits",
        "review proposals proposal apply works changes",
    ),
    (
        Setting::Folders,
        PreferencesPane::Agent,
        "Folders",
        "folder add remove access media read save directories roots permissions",
    ),
];

/// A group of the Keyboard pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keys {
    Playback,
    Editing,
    Timeline,
    Window,
}

impl Keys {
    const ALL: [Self; 4] = [Self::Playback, Self::Editing, Self::Timeline, Self::Window];

    fn name(self) -> &'static str {
        match self {
            Self::Playback => "Playback",
            Self::Editing => "Editing",
            Self::Timeline => "Timeline",
            Self::Window => "Project and window",
        }
    }
}

/// A row of the Keyboard pane: what a chord does and the keys, written as
/// the menus write them, with "Cmd" for the platform's command key. Chords
/// that differ on macOS beyond that say how they go there.
struct Chord {
    group: Keys,
    what: &'static str,
    keys: &'static str,
    mac: Option<&'static str>,
}

const fn chord(group: Keys, what: &'static str, keys: &'static str) -> Chord {
    Chord {
        group,
        what,
        keys,
        mac: None,
    }
}

const SHORTCUTS: [Chord; 35] = [
    chord(Keys::Playback, "Play or pause", "Space"),
    chord(Keys::Playback, "Previous or next frame", "← →"),
    chord(Keys::Playback, "Back or forward one second", "Shift+← →"),
    chord(Keys::Playback, "Start or end", "Home End"),
    chord(
        Keys::Playback,
        "Back one second, stop, play faster",
        "J K L",
    ),
    chord(Keys::Editing, "Select the clip under the playhead", "C"),
    chord(Keys::Editing, "Select the clip before or after", "Alt+← →"),
    chord(
        Keys::Editing,
        "Select the clip on the track above or below",
        "Alt+↑ ↓",
    ),
    chord(Keys::Editing, "Split at playhead", "S"),
    chord(Keys::Editing, "Delete", "Delete"),
    chord(Keys::Editing, "Ripple delete", "Shift+Delete"),
    chord(Keys::Editing, "Undo", "Cmd+Z"),
    chord(Keys::Editing, "Redo", "Shift+Cmd+Z"),
    chord(Keys::Editing, "Cut, copy, paste", "Cmd+X C V"),
    chord(Keys::Editing, "Duplicate", "Cmd+D"),
    chord(Keys::Editing, "Select all", "Cmd+A"),
    chord(Keys::Editing, "Group, ungroup", "Cmd+G, Shift+Cmd+G"),
    chord(Keys::Timeline, "Add marker", "M"),
    chord(Keys::Editing, "Add text", "T"),
    chord(Keys::Editing, "Add transition", "Cmd+T"),
    chord(Keys::Editing, "Freeze frame", "F"),
    chord(Keys::Timeline, "Keyframes of the clip", "U"),
    chord(Keys::Timeline, "Graph editor", "G"),
    chord(Keys::Timeline, "Snapping", "N"),
    chord(Keys::Timeline, "Zoom timeline", "Cmd+Scroll, = -"),
    chord(Keys::Timeline, "Fit timeline", "Shift+Z"),
    chord(Keys::Window, "Ask the agent", "Shift+Cmd+K"),
    chord(Keys::Window, "Command line", "Cmd+K"),
    chord(Keys::Window, "Save", "Cmd+S"),
    chord(Keys::Window, "Import media", "Cmd+I"),
    chord(Keys::Window, "Export", "Cmd+E"),
    chord(Keys::Window, "Render queue", "Shift+Cmd+E"),
    chord(Keys::Window, "Interface size", "Cmd+= Cmd+- Cmd+0"),
    Chord {
        group: Keys::Window,
        what: "Show or hide the library",
        keys: "Shift+Cmd+B",
        mac: Some("Cmd+B"),
    },
    Chord {
        group: Keys::Window,
        what: "Show or hide the inspector",
        keys: "Shift+Cmd+O",
        mac: Some("Alt+Cmd+O"),
    },
];

impl Chord {
    /// The keys on this platform, as the menus write them.
    fn keys(&self) -> &'static str {
        match self.mac {
            Some(mac) if cfg!(target_os = "macos") => mac,
            _ => self.keys,
        }
    }
}

/// How many slips separate two short words: a letter added, dropped or
/// replaced, or two neighbours swapped.
fn edits(a: &[char], b: &[char]) -> usize {
    let width = b.len() + 1;
    let mut cost: Vec<usize> = (0..(a.len() + 1) * width).map(|i| i % width).collect();
    for i in 1..=a.len() {
        cost[i * width] = i;
        for j in 1..=b.len() {
            let mut best = (cost[(i - 1) * width + j - 1] + usize::from(a[i - 1] != b[j - 1]))
                .min(cost[(i - 1) * width + j] + 1)
                .min(cost[i * width + j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(cost[(i - 2) * width + j - 2] + 1);
            }
            cost[i * width + j] = best;
        }
    }
    cost[a.len() * width + b.len()]
}

/// Whether `typed` is `word`, or the start of it, give or take a slip of the
/// keys. Short words must be typed exactly: a slip there is another word.
fn nearly(typed: &str, word: &str) -> bool {
    let typed: Vec<char> = typed.chars().collect();
    let word: Vec<char> = word.chars().collect();
    let allowed = match typed.len() {
        0..=3 => return false,
        4..=7 => 1,
        _ => 2,
    };
    (typed.len().saturating_sub(allowed)..=typed.len() + allowed)
        .filter(|&length| length <= word.len())
        .any(|length| edits(&typed, &word[..length]) <= allowed)
}

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// Every setting a search reads, with its pane and the words it is found by.
fn searched() -> impl Iterator<Item = (Setting, PreferencesPane, Vec<String>)> {
    let settings = INDEX.iter().map(|&(setting, pane, label, terms)| {
        let known = words(pane.name())
            .chain(words(label))
            .chain(words(terms))
            .collect();
        (setting, pane, known)
    });
    let chords = SHORTCUTS.iter().enumerate().map(|(index, chord)| {
        let pane = PreferencesPane::Keyboard;
        let known = words(pane.name())
            .chain(words(chord.what))
            .chain(words(chord.group.name()))
            .chain(words("shortcut shortcuts keys chord chords hotkey"))
            .collect();
        (Setting::Shortcut(index), pane, known)
    });
    settings.chain(chords)
}

/// The settings a query finds, in the order of the panes, or `None` when
/// there is nothing to search for. Every word typed must be found in a
/// setting: at the start of one of its words, or inside one once three
/// letters are typed. Misspellings count only when nothing matches as typed.
fn search(query: &str) -> Option<Vec<Setting>> {
    let typed: Vec<String> = words(query).collect();
    if typed.is_empty() {
        return None;
    }
    let mut exact = Vec::new();
    let mut near = Vec::new();
    for (setting, _, known) in searched() {
        let found = |typed: &String| {
            known.iter().any(|word| {
                word.starts_with(typed.as_str())
                    || (typed.chars().count() >= 3 && word.contains(typed.as_str()))
            })
        };
        if typed.iter().all(found) {
            exact.push(setting);
        } else if typed
            .iter()
            .all(|typed| found(typed) || known.iter().any(|word| nearly(typed, word)))
        {
            near.push(setting);
        }
    }
    Some(if exact.is_empty() { near } else { exact })
}

fn pane_of(setting: Setting) -> PreferencesPane {
    match setting {
        Setting::Shortcut(_) => PreferencesPane::Keyboard,
        setting => (INDEX.iter())
            .find(|entry| entry.0 == setting)
            .map_or(PreferencesPane::Appearance, |entry| entry.1),
    }
}

/// Which settings a pane draws: all of its own, or those a search found.
struct Listed<'a> {
    found: Option<&'a [Setting]>,
    cards: Cell<usize>,
}

impl Listed<'_> {
    fn has(&self, setting: Setting) -> bool {
        self.found.is_none_or(|found| found.contains(&setting))
    }
}

/// A titled card holding those of `members` that are shown, or nothing when
/// none are. Search results name the pane each card comes from.
fn card(
    ui: &mut Ui,
    p: &Palette,
    listed: &Listed,
    pane: PreferencesPane,
    title: &str,
    members: &[Setting],
    add_rows: impl FnOnce(&mut Ui, &mut Group),
) {
    if !members.iter().any(|&setting| listed.has(setting)) {
        return;
    }
    if listed.cards.replace(listed.cards.get() + 1) > 0 {
        // Cards stand 14 apart, as Terminal's Preferences cards do (§5.9.2).
        ui.add_space(14.0);
    }
    // Every card is titled, as Terminal's are, and never with its pane's
    // own name, which the header already says; a search says whose each
    // card is.
    let label = match listed.found {
        Some(_) => format!("{} · {title}", pane.name()),
        None => title.to_owned(),
    };
    let at = ui.cursor().min;
    controls::section_label(ui, *p, &label);
    said(
        ui,
        painted(
            ui,
            pos2(at.x + 4.0, at.y + 12.0),
            &label,
            theme::medium(11.5),
        ),
        &label,
    );
    controls::group(ui, *p, add_rows);
}

/// Where words painted from the middle of their leading edge at `at` are.
fn painted(ui: &Ui, at: Pos2, words: &str, font: egui::FontId) -> Rect {
    let size = (ui.painter())
        .layout_no_wrap(words.to_owned(), font, Color32::PLACEHOLDER)
        .size();
    Rect::from_min_size(pos2(at.x, at.y - size.y * 0.5), size)
}

/// Tells assistive tools of words that are only painted, at `rect`, as the
/// label they were before Terminal's rows drew them by hand.
fn said(ui: &Ui, rect: Rect, words: &str) {
    ui.interact(rect, ui.auto_id_with(("said", words)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, words));
}

/// A row of a Preferences card, as tall as Terminal's, its label said too.
fn row<R>(ui: &mut Ui, rows: &mut Group, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    row_of(ui, rows, label, metrics::PREFERENCES_ROW, add)
}

/// A row as `row` draws it, `height` tall.
fn row_of<R>(
    ui: &mut Ui,
    rows: &mut Group,
    label: &str,
    height: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let at = ui.cursor().min;
    let words = painted(
        ui,
        pos2(at.x + 14.0, at.y + height * 0.5),
        label,
        theme::regular(13.0),
    );
    said(ui, words, label);
    rows.row_with_height(ui, label, height, add)
}

/// A note of a card as `Group::note` draws it, its words said too.
fn note(ui: &mut Ui, rows: &mut Group, text: &str) {
    let at = ui.cursor().min;
    let width = ui.available_width();
    rows.note(ui, text);
    let bottom = ui.min_rect().bottom();
    said(
        ui,
        Rect::from_min_max(
            pos2(at.x + 14.0, at.y + 10.0),
            pos2(at.x + width - 14.0, bottom - 10.0),
        ),
        text,
    );
}

/// Terminal's segmented control, whose options Left and Right also choose
/// while one of them has the keyboard (Terminal behaviour). Returns whether
/// the choice changed.
fn choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    name: &str,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    // The ids `controls::segmented` gives its cells.
    let id = ui.id().with(("segmented", name));
    let cells: Vec<Id> = (options.iter()).map(|(_, label)| id.with(*label)).collect();
    let selected = (options.iter())
        .position(|(option, _)| option == value)
        .unwrap_or(0);
    let arrowed = super::arrow_choice(ui.ctx(), &cells, selected);
    if let Some(to) = arrowed {
        *value = options[to].0;
    }
    controls::segmented(ui, *p, name, value, options, width) || arrowed.is_some()
}

/// The proxy cache sizes offered, in GiB.
const PROXY_BUDGETS: [u32; 7] = [5, 10, 20, 50, 100, 200, 500];

/// What the note under a System appearance says: that the system's light or
/// dark mode chooses, or that the system has not said, so the window is
/// dark (`Appearance::resolve`).
fn system_note(reported: bool) -> &'static str {
    if reported {
        "Your system's light or dark mode chooses the appearance of the window."
    } else {
        "Your system has not reported light or dark mode, so Neptune Video is dark."
    }
}

/// What a note under a control says of each quality.
fn quality_note(quality: PreviewQuality) -> &'static str {
    match quality {
        PreviewQuality::Auto => "Full detail, and half while playback cannot keep up.",
        PreviewQuality::Full => "Every pixel the screen shows, decoded and composited.",
        PreviewQuality::Half => "Half the detail, for smoother playback.",
        PreviewQuality::Quarter => "A quarter of the detail, for the smoothest playback.",
    }
}

/// How much the proxy cache holds of its budget, as "3.2 GB of 20 GB".
/// Gigabytes here are those of the budget: 2³⁰ bytes.
pub(crate) fn cache_usage(bytes: u64, budget_gib: u32) -> String {
    const MIB: f64 = (1u64 << 20) as f64;
    const GIB: f64 = (1u64 << 30) as f64;
    let held = bytes as f64;
    let used = if bytes == 0 {
        "Nothing".to_owned()
    } else if held >= GIB / 10.0 {
        format!("{:.1} GB", held / GIB)
    } else {
        format!("{:.0} MB", (held / MIB).max(1.0))
    };
    format!("{used} of {budget_gib} GB")
}

/// Preferences: a source list of panes whose settings apply as they change.
/// `cache` is what the proxy cache holds now. Returns whether it should
/// close. The pane in view is the one `PreferencesPane` keeps for the
/// window's session.
pub fn settings(
    ctx: &egui::Context,
    p: &Palette,
    settings: &Settings,
    cache: Cache,
    actions: &mut Vec<Action>,
) -> bool {
    let screen = ctx.content_rect();
    let query_id = Id::new("preferences-query");
    // A search is left behind when Preferences closes.
    let mut query = if opening(ctx, Id::new("settings-sheet")) {
        String::new()
    } else {
        ctx.data(|data| data.get_temp::<String>(query_id))
            .unwrap_or_default()
    };
    // Escape leaves a search before it leaves the sheet, but only once what
    // is inside it is closed: an open list, or a field other than the
    // search that had the keyboard, takes that Escape for itself.
    let elsewhere_id = Id::new("preferences-typing-elsewhere");
    let inner_open = egui::Popup::is_any_open(ctx)
        || ctx.data(|data| data.get_temp::<bool>(elsewhere_id)) == Some(true);
    let escape_taken =
        !query.is_empty() && !inner_open && ctx.input(|input| input.key_pressed(Key::Escape));
    if escape_taken {
        query.clear();
    }
    let mut pane = PreferencesPane::shown(ctx);
    let mut close = false;
    let heard = scaffold(
        ctx,
        p,
        "settings-sheet",
        "Preferences",
        PREFERENCES_WIDTH,
        escape_taken,
        |ui| {
            // Leave the window visible around the sheet when there is room;
            // a short window gives most of that margin back to the settings.
            let chrome = HEADER + 60.0;
            let body_height = (screen.height() - chrome - 96.0)
                .max((screen.height() - chrome - 24.0).min(260.0))
                .clamp(120.0, 560.0);
            // One height for every pane, so choosing one never moves the sheet.
            let height = (body_height + chrome).min(PREFERENCES_HEIGHT);
            let (_, rect) = ui.allocate_space(vec2(ui.available_width(), height));
            let compact = rect.width() < COMPACT_BELOW;
            let list = Rect::from_min_size(
                rect.min + Vec2::splat(INSET),
                vec2(if compact { RAIL } else { SIDEBAR }, height - INSET * 2.0),
            );
            let content = Rect::from_min_max(pos2(list.right(), rect.top()), rect.max);
            // Concentric with the sheet's corner.
            let radius = metrics::SHEET_RADIUS - INSET as u8;
            ui.painter().rect_filled(list, radius, p.chrome);
            ui.painter()
                .rect_stroke(list, radius, p.hairline(), StrokeKind::Inside);

            // Beside the names while there is room; otherwise the search
            // titles the content, and the rail says which pane is in view.
            let (field, panes) = if compact {
                (
                    Rect::from_min_max(
                        pos2(content.left() + 20.0, content.top() + 11.0),
                        pos2(content.right() - 50.0, content.top() + 41.0),
                    ),
                    list.with_min_y(list.top() + INSET),
                )
            } else {
                let top = list.top() + HEADER - INSET;
                let at = pos2(list.left() + 14.0, (list.top() + top) * 0.5);
                ui.painter().text(
                    at,
                    Align2::LEFT_CENTER,
                    "Preferences",
                    theme::semibold(15.0),
                    p.fg,
                );
                let words = painted(ui, at, "Preferences", theme::semibold(15.0));
                said(ui, words, "Preferences");
                (
                    Rect::from_min_size(
                        pos2(list.left() + 8.0, top),
                        vec2(list.width() - 16.0, metrics::CONTROL_HEIGHT),
                    ),
                    list.with_min_y(top + metrics::CONTROL_HEIGHT + 8.0),
                )
            };
            let searched = controls::place(
                ui,
                field,
                Layout::left_to_right(Align::Center),
                "preferences-search",
                |ui| {
                    controls::search_field(
                        ui,
                        *p,
                        Id::new("preferences-search"),
                        &mut query,
                        "Search",
                        "Search settings",
                        field.width(),
                    )
                },
            );
            let mut found = search(&query);
            // Enter opens the pane of the first setting found.
            if searched.lost_focus()
                && ui.input(|input| input.key_pressed(Key::Enter))
                && let Some(&first) = found.as_ref().and_then(|found| found.first())
            {
                pane = pane_of(first);
                query.clear();
                found = None;
            }

            controls::place(
                ui,
                panes,
                Layout::top_down(Align::Min),
                "preferences-sidebar",
                |ui| sidebar(ui, p, &mut pane, &mut query, found.as_deref(), compact),
            );
            // Choosing a pane leaves the search.
            if query.is_empty() {
                found = None;
            }
            controls::place(
                ui,
                content,
                Layout::top_down(Align::Min),
                "preferences-pane",
                |ui| {
                    let title = match (compact, &found) {
                        (true, _) => "",
                        (false, Some(_)) => "Results",
                        (false, None) => pane.name(),
                    };
                    let at = ui.cursor().min;
                    close |= controls::sheet_header(ui, *p, title, Some("Close preferences"));
                    title_said(ui, at, title);
                    if found.as_ref().is_some_and(Vec::is_empty) {
                        nothing_found(ui, p, height - HEADER);
                        return;
                    }
                    let listed = Listed {
                        found: found.as_deref(),
                        cards: Cell::new(0),
                    };
                    // A search lists every pane; otherwise the one in view.
                    let shows = |shown: PreferencesPane| listed.found.is_some() || shown == pane;
                    let scrolled = egui::ScrollArea::vertical()
                        .id_salt((
                            "preferences-body",
                            listed.found.is_none().then_some(pane.name()),
                        ))
                        .max_height(height - HEADER)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            controls::padded(ui, 20.0, |ui| {
                                if shows(PreferencesPane::Appearance) {
                                    appearance(ui, p, &listed, settings, actions);
                                }
                                if shows(PreferencesPane::Editing) {
                                    editing(ui, p, &listed, settings, actions);
                                }
                                if shows(PreferencesPane::Preview) {
                                    preview(ui, p, &listed, &settings.preview, cache, actions);
                                }
                                if shows(PreferencesPane::Agent) {
                                    agent(ui, p, &listed, settings, actions);
                                }
                                if shows(PreferencesPane::Keyboard) {
                                    keyboard(ui, p, &listed);
                                }
                                ui.add_space(20.0);
                            });
                        });
                    fade_below(ui, p, &scrolled);
                },
            );
        },
    );
    pane.show(ctx);
    // Escape takes the keyboard out of a field before the next frame shows,
    // so whether one other than the search had it is kept from this one.
    let elsewhere = ctx
        .memory(|memory| memory.focused())
        .is_some_and(|focused| {
            focused != controls::viewport_id(ctx, Id::new("preferences-search"))
                && egui::text_edit::TextEditState::load(ctx, focused).is_some()
        });
    ctx.data_mut(|data| {
        data.insert_temp(query_id, query);
        data.insert_temp(elsewhere_id, elsewhere);
    });
    close || heard.dismissed
}

/// What the content says when a search finds no setting.
fn nothing_found(ui: &mut Ui, p: &Palette, height: f32) {
    let (_, area) = ui.allocate_space(vec2(ui.available_width(), height));
    kit::empty_state(
        ui,
        p,
        area,
        Icon::Search,
        "No settings found",
        "Try another word for it.",
        None,
    );
}

/// The source list: the panes by name, or by icon alone in a narrow sheet.
/// During a search each pane counts what was found in it.
fn sidebar(
    ui: &mut Ui,
    p: &Palette,
    pane: &mut PreferencesPane,
    query: &mut String,
    found: Option<&[Setting]>,
    compact: bool,
) {
    let width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("preferences-panes")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entry in PreferencesPane::ALL {
                let (_, slot) = ui.allocate_space(vec2(width, 32.0));
                let row = slot.shrink2(vec2(INSET, 1.0));
                let response = ui.interact(
                    row,
                    Id::new(("preferences-pane", entry.name())),
                    Sense::click(),
                );
                if response.clicked() {
                    *pane = entry;
                    query.clear();
                }
                let matches = found.map(|found| {
                    (found.iter())
                        .filter(|&&setting| pane_of(setting) == entry)
                        .count()
                });
                // Results take the content; no single pane is in view then.
                let selected = matches.is_none() && *pane == entry;
                response.widget_info(|| {
                    WidgetInfo::selected(WidgetType::RadioButton, true, selected, entry.name())
                });
                let painter = ui.painter();
                if selected || response.is_pointer_button_down_on() {
                    painter.rect_filled(row, metrics::ROW_RADIUS, p.pressed);
                } else if response.hovered() {
                    painter.rect_filled(row, metrics::ROW_RADIUS, p.hover);
                }
                if response.has_focus() {
                    controls::focus_ring(painter, row.shrink(2.0), metrics::ROW_RADIUS - 2, *p);
                }
                let ink = if selected || response.hovered() {
                    p.fg
                } else if matches == Some(0) {
                    p.muted
                } else {
                    p.secondary
                };
                let icon = if compact {
                    row.center()
                } else {
                    pos2(row.left() + 16.0, row.center().y)
                };
                icons::paint(
                    painter,
                    Rect::from_center_size(icon, Vec2::splat(16.0)),
                    pane_icon(entry),
                    ink,
                );
                if !compact {
                    let mut trailing = row.right() - 10.0;
                    if let Some(count) = matches.filter(|&count| count > 0) {
                        let count =
                            painter.layout_no_wrap(count.to_string(), theme::medium(11.5), ink);
                        trailing -= count.size().x;
                        controls::galley_at(painter, pos2(trailing, row.center().y), count);
                        trailing -= 6.0;
                    }
                    let left = row.left() + 32.0;
                    controls::galley_at(
                        painter,
                        pos2(left, row.center().y),
                        controls::elided(
                            painter,
                            entry.name(),
                            theme::medium(13.0),
                            ink,
                            trailing - left,
                        ),
                    );
                }
                let response = response.on_hover_cursor(CursorIcon::PointingHand);
                if compact {
                    response.on_hover_text(entry.name());
                }
            }
        });
}

/// Appearance: the look of the window, its size and its motion.
fn appearance(
    ui: &mut Ui,
    p: &Palette,
    listed: &Listed,
    settings: &Settings,
    actions: &mut Vec<Action>,
) {
    let pane = PreferencesPane::Appearance;
    card(
        ui,
        p,
        listed,
        pane,
        "Window",
        &[Setting::Appearance, Setting::Accent, Setting::InterfaceSize],
        |ui, rows| {
            if listed.has(Setting::Appearance) {
                row(ui, rows, "Appearance", |ui| {
                    let mut appearance = settings.appearance;
                    if choice(
                        ui,
                        p,
                        "Appearance",
                        &mut appearance,
                        &[
                            (Appearance::Light, "Light"),
                            (Appearance::Dark, "Dark"),
                            (Appearance::System, "System"),
                        ],
                        228.0,
                    ) {
                        actions.push(Action::SetAppearance(appearance));
                    }
                });
                // Which the system chose, or that it said nothing, as
                // Terminal tells it.
                if settings.appearance == Appearance::System {
                    let reported = ui.ctx().system_theme().is_some();
                    note(ui, rows, system_note(reported));
                }
            }
            if listed.has(Setting::Accent) {
                row(ui, rows, "Accent", |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // Laid out from the trailing edge: Blue leads.
                    for (accent, name) in Accent::ALL.into_iter().rev() {
                        if swatch(ui, p, accent, name, settings.accent == accent)
                            && settings.accent != accent
                        {
                            actions.push(Action::SetAccent(accent));
                        }
                    }
                });
            }
            if listed.has(Setting::InterfaceSize) {
                row(ui, rows, "Interface size", |ui| {
                    if let Some(to) = size_stepper(ui, p, settings.interface_scale) {
                        actions.push(Action::ScaleInterface(to));
                    }
                });
            }
        },
    );
    card(
        ui,
        p,
        listed,
        pane,
        "Motion and layout",
        &[Setting::ReduceMotion, Setting::ResetLayout],
        |ui, rows| {
            if listed.has(Setting::ReduceMotion) {
                row(ui, rows, "Reduce motion", |ui| {
                    let mut on = settings.reduce_motion;
                    if controls::toggle(ui, *p, &mut on, "Reduce motion").changed() {
                        actions.push(Action::SetReduceMotion(on));
                    }
                });
            }
            if listed.has(Setting::ResetLayout) {
                row(ui, rows, "Layout", |ui| {
                    let reset = controls::button(ui, *p, "Reset layout", ButtonKind::Secondary)
                        .on_hover_text(
                            "Show the library and the inspector again at their first widths, \
                             over a timeline of its first height",
                        );
                    if reset.clicked() {
                        actions.push(Action::ResetLayout);
                    }
                });
            }
        },
    );
}

/// One accent as a coloured disc, as Neptune Finder offers it. The one
/// chosen is ringed in the ink of the text, three points out, so that it
/// reads whatever its colour.
fn swatch(ui: &mut Ui, p: &Palette, accent: Accent, name: &str, selected: bool) -> bool {
    let (_, rect) = ui.allocate_space(Vec2::splat(20.0));
    let response = ui.interact(rect, ui.id().with(("accent", name)), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, name));
    let painter = ui.painter();
    let fill = accent.color(p.dark);
    let center = rect.center();
    painter.circle_filled(center, 7.5, fill);
    painter.circle_stroke(center, 7.5, Stroke::new(0.5, Color32::from_black_alpha(40)));
    if selected {
        painter.circle_stroke(center, 10.5, Stroke::new(2.0, p.fg));
    } else if response.hovered() {
        painter.circle_stroke(center, 9.5, Stroke::new(1.0, p.border));
    }
    if response.has_focus() {
        painter.circle_stroke(center, 13.0, Stroke::new(2.0, theme::tint(p.accent, 0.55)));
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(name)
        .clicked()
}

/// The interface size as Terminal's stepper, 112 by 28: Smaller, the size,
/// which goes back to 100%, and Larger. A step that would change nothing is
/// not offered, and the size shown is the one the window holds, which may
/// be under the one asked for. Returns the size asked for.
fn size_stepper(ui: &mut Ui, p: &Palette, asked: f32) -> Option<f32> {
    let room = super::largest_scale(ui.ctx());
    let scale = asked.min(room);
    let (_, rect) = ui.allocate_space(vec2(112.0, 28.0));
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 8, p.control);
    let mut asked = None;
    // `label` names the cell; `tip` says what it does, keys and all.
    let mut cell = |rect: Rect, label: &str, tip: &str, to: f32, icon: Option<Icon>| {
        let enabled = to != scale;
        let response = ui.interact(rect, ui.id().with(("interface-size", tip)), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
        let surface = rect.shrink(2.0);
        if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(surface, 6, p.pressed);
        } else if enabled && response.hovered() {
            painter.rect_filled(surface, 6, p.hover);
        }
        if response.has_focus() {
            controls::focus_ring(&painter, surface, 6, *p);
        }
        let ink = if !enabled {
            p.muted
        } else if response.hovered() {
            p.fg
        } else {
            p.secondary
        };
        match icon {
            Some(icon) => icons::paint(
                &painter,
                Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
                icon,
                ink,
            ),
            // The size itself.
            None => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    theme::medium(12.5),
                    p.fg,
                );
            }
        }
        if response.clicked() && enabled {
            asked = Some(to);
        }
        if enabled {
            let (name, keys) = kit::name_and_keys(tip);
            icons::hint(
                response.on_hover_cursor(CursorIcon::PointingHand),
                name,
                keys,
            );
        }
    };
    let left = Rect::from_min_size(rect.min, vec2(30.0, rect.height()));
    let right = Rect::from_min_size(pos2(rect.right() - 30.0, rect.top()), left.size());
    let middle = Rect::from_min_max(
        pos2(left.right(), rect.top()),
        pos2(right.left(), rect.bottom()),
    );
    let smaller = shortcut("Smaller (Cmd+-)");
    cell(
        left,
        &smaller,
        &smaller,
        Scale::Smaller.applied_to(scale).min(room),
        Some(Icon::Minus),
    );
    // Today's format, as the size was always written.
    cell(
        middle,
        &format!("{:.0}%", scale * 100.0),
        &shortcut("Actual size (Cmd+0)"),
        Scale::Actual.applied_to(scale),
        None,
    );
    let larger = shortcut("Larger (Cmd+=)");
    cell(
        right,
        &larger,
        &larger,
        Scale::Larger.applied_to(scale).min(room),
        Some(Icon::Plus),
    );
    asked
}

/// Editing: what dragging does, and the copy kept of unsaved work.
fn editing(
    ui: &mut Ui,
    p: &Palette,
    listed: &Listed,
    settings: &Settings,
    actions: &mut Vec<Action>,
) {
    card(
        ui,
        p,
        listed,
        PreferencesPane::Editing,
        "While editing",
        &[Setting::Snapping, Setting::RecoveryCopy],
        |ui, rows| {
            if listed.has(Setting::Snapping) {
                row(ui, rows, "Snap while dragging", |ui| {
                    let mut on = settings.snapping;
                    if controls::toggle(ui, *p, &mut on, "Snap while dragging").changed() {
                        actions.push(Action::ToggleSnapping);
                    }
                });
            }
            if listed.has(Setting::RecoveryCopy) {
                row(ui, rows, "Keep a recovery copy while editing", |ui| {
                    let mut on = settings.autosave;
                    if controls::toggle(ui, *p, &mut on, "Keep a recovery copy").changed() {
                        actions.push(Action::SetAutosave(on));
                    }
                });
            }
        },
    );
}

/// Preview: how much of the pictures the preview decodes, and the proxies
/// it may read in their place.
fn preview(
    ui: &mut Ui,
    p: &Palette,
    listed: &Listed,
    preview: &PreviewSettings,
    cache: Cache,
    actions: &mut Vec<Action>,
) {
    let pane = PreferencesPane::Preview;
    card(
        ui,
        p,
        listed,
        pane,
        "Quality",
        &[Setting::Quality],
        |ui, rows| {
            row(ui, rows, "Preview quality", |ui| {
                let mut quality = preview.quality;
                let options = PreviewQuality::ALL.map(|quality| (quality, quality.name()));
                if choice(ui, p, "Preview quality", &mut quality, &options, 236.0) {
                    actions.push(Action::SetPreviewQuality(quality));
                }
            });
            note(ui, rows, quality_note(preview.quality));
        },
    );
    card(
        ui,
        p,
        listed,
        pane,
        "Proxy media",
        &[Setting::Proxies, Setting::ProxyCache],
        |ui, rows| {
            if listed.has(Setting::Proxies) {
                row(ui, rows, "Proxies", |ui| {
                    let mut mode = preview.proxies;
                    let options = ProxyMode::ALL.map(|mode| (mode, mode.name()));
                    if choice(ui, p, "Proxies", &mut mode, &options, 180.0) {
                        actions.push(Action::SetProxyMode(mode));
                    }
                });
                note(
                    ui,
                    rows,
                    "Proxies are smaller copies used only for preview; exports always use the \
                     original media.",
                );
            }
            if !cache.available {
                note(ui, rows, "Proxies are unavailable on this computer.");
                return;
            }
            if !listed.has(Setting::ProxyCache) {
                return;
            }
            row(ui, rows, "Cache size", |ui| {
                let chosen = preview.proxy_budget_gib;
                // A size kept from elsewhere stays offered.
                let sizes: Vec<(u32, String)> = (!PROXY_BUDGETS.contains(&chosen))
                    .then_some(chosen)
                    .into_iter()
                    .chain(PROXY_BUDGETS)
                    .map(|gib| (gib, format!("{gib} GB")))
                    .collect();
                let options: Vec<(u32, &str)> = (sizes.iter())
                    .map(|(gib, name)| (*gib, name.as_str()))
                    .collect();
                let mut pick = chosen;
                let list = kit::select_list(
                    ui,
                    p,
                    Id::new("proxy-budget"),
                    "Proxy cache size",
                    &mut pick,
                    &options,
                    vec2(180.0, metrics::CONTROL_HEIGHT),
                );
                if list.changed() {
                    actions.push(Action::SetProxyBudget(pick));
                }
            });
            let usage = cache_usage(cache.bytes, preview.proxy_budget_gib);
            row(ui, rows, &usage, |ui| {
                let empty = cache.proxies == 0;
                let clear = ui
                    .add_enabled_ui(!empty, |ui| {
                        controls::button(ui, *p, "Clear proxies", ButtonKind::Secondary)
                    })
                    .inner
                    .on_hover_text(
                        "Remove every proxy. None is made again until you ask for one or \
                         reopen the project.",
                    )
                    .on_disabled_hover_text("There are no proxies to clear");
                if clear.clicked() {
                    actions.push(Action::ClearProxies);
                }
            });
        },
    );
}

/// Agent: who answers, how, and the folders it may use.
fn agent(
    ui: &mut Ui,
    p: &Palette,
    listed: &Listed,
    settings: &Settings,
    actions: &mut Vec<Action>,
) {
    let pane = PreferencesPane::Agent;
    card(
        ui,
        p,
        listed,
        pane,
        "Requests",
        &[
            Setting::Provider,
            Setting::Model,
            Setting::Effort,
            Setting::Edits,
        ],
        |ui, rows| {
            if listed.has(Setting::Provider) {
                row(ui, rows, "Answered by", |ui| {
                    let mut provider = settings.ai.provider;
                    let options = Provider::ALL.map(|provider| (provider, provider.name()));
                    if choice(ui, p, "Answered by", &mut provider, &options, 150.0) {
                        actions.push(Action::SetAiProvider(provider));
                    }
                });
            }
            if listed.has(Setting::Model) {
                row(ui, rows, "Model", |ui| {
                    let mut model = settings.ai.model().to_owned();
                    let field = controls::text_field(
                        ui,
                        *p,
                        Id::new("agent-model"),
                        &mut model,
                        "Provider's default",
                        "Model",
                        180.0,
                    );
                    if field.changed() {
                        actions.push(Action::SetAiModel(model));
                    }
                });
            }
            if listed.has(Setting::Effort) {
                row(ui, rows, "Effort", |ui| {
                    let mut effort = settings.ai.effort;
                    let options = Effort::ALL.map(|effort| (effort, effort.name()));
                    if choice(ui, p, "Effort", &mut effort, &options, 236.0) {
                        actions.push(Action::SetAiEffort(effort));
                    }
                });
            }
            if listed.has(Setting::Edits) {
                row(ui, rows, "Edits", |ui| {
                    let mut review = settings.ai.review;
                    if choice(
                        ui,
                        p,
                        "Edits",
                        &mut review,
                        &[(false, "As it works"), (true, "For review")],
                        190.0,
                    ) {
                        actions.push(Action::SetAiReview(review));
                    }
                });
            }
            note(
                ui,
                rows,
                "Requests go through the Claude or Codex command line installed on this \
                 computer, with the sign-in it already has. Neptune Video keeps no key or \
                 password.",
            );
            // What the agent may do and see, as it always said, one click away.
            let open_id = Id::new("preferences-agent-sees");
            let mut open = ui.ctx().data(|data| data.get_temp::<bool>(open_id)) == Some(true);
            if rows
                .disclosure(ui, "What the agent can see", &mut open)
                .changed()
            {
                ui.ctx().data_mut(|data| data.insert_temp(open_id, open));
            }
            if open {
                note(
                    ui,
                    rows,
                    "As it works, the agent edits with every tool an outside agent has, and \
                     each edit is a step you can undo. It may use the project's folders, your \
                     Movies, Music, Pictures, Desktop, Documents and Downloads folders, and \
                     those added here. For review, it proposes edits from the project's data \
                     alone, and nothing changes until you apply them.",
                );
            }
        },
    );
    card(
        ui,
        p,
        listed,
        pane,
        "Folders",
        &[Setting::Folders],
        |ui, rows| {
            // Like --root for an outside agent: folders the agent may use
            // besides the project's and the person's usual places.
            for (index, folder) in settings.ai.folders.iter().enumerate() {
                let name = folder.file_name().map_or_else(
                    || folder.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                row(ui, rows, &name, |ui| {
                    // Two folders may share a name; each Remove is its own.
                    ui.push_id(("agent-folder", index), |ui| {
                        let remove = controls::button(ui, *p, "Remove", ButtonKind::Quiet)
                            .on_hover_text(folder.display().to_string());
                        if remove.clicked() {
                            actions.push(Action::RemoveAgentFolder(index));
                        }
                    });
                });
            }
            row(ui, rows, "Let the agent use a folder", |ui| {
                let add = controls::button(ui, *p, "Add folder…", ButtonKind::Secondary)
                    .on_hover_text(
                        "Let the agent read media from and save to this folder, besides the \
                         project's folders and your usual ones",
                    );
                if add.clicked() {
                    actions.push(Action::AddAgentFolder);
                }
            });
        },
    );
}

/// Keyboard: every chord, grouped, with its keys as keycaps. Read only.
fn keyboard(ui: &mut Ui, p: &Palette, listed: &Listed) {
    for group in Keys::ALL {
        let members: Vec<Setting> = (SHORTCUTS.iter().enumerate())
            .filter(|(_, chord)| chord.group == group)
            .map(|(index, _)| Setting::Shortcut(index))
            .collect();
        card(
            ui,
            p,
            listed,
            PreferencesPane::Keyboard,
            group.name(),
            &members,
            |ui, rows| {
                for setting in &members {
                    let Setting::Shortcut(index) = *setting else {
                        continue;
                    };
                    if !listed.has(*setting) {
                        continue;
                    }
                    let chord = &SHORTCUTS[index];
                    row_of(ui, rows, chord.what, 32.0, |ui| {
                        let room = ui.max_rect();
                        // The keys are what the row says, so they are ink
                        // to read rather than a hint beside a command.
                        let center = pos2(room.right(), room.center().y);
                        let width = keys(ui.painter(), center, chord.keys(), p.secondary);
                        // Said as the menus write them, apart from what
                        // they do, as they always were.
                        let written = shortcut(chord.keys());
                        let caps = Rect::from_x_y_ranges(
                            (center.x - width)..=center.x,
                            (center.y - 9.0)..=(center.y + 9.0),
                        );
                        let response =
                            ui.interact(caps, ui.id().with(("chord", index)), Sense::hover());
                        response
                            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &written));
                    });
                }
            },
        );
    }
}

/// Paints the keys of a row as keycaps, right-aligned at `right_center`:
/// chords apart from one another by more than the keys of one chord, and
/// alternatives, written after a comma, set apart by a slash. The command
/// key is "Ctrl" here and ⌘ on macOS, where Shift and Alt are ⇧ and ⌥, and
/// a minus is the sign rather than a hyphen, which a cap shows as a dash.
/// Returns how wide the keys are.
fn keys(painter: &egui::Painter, right_center: Pos2, keys: &str, ink: Color32) -> f32 {
    let mac = cfg!(target_os = "macos");
    let cap = |key: &str| -> String {
        match key {
            "Cmd" if mac => "⌘".to_owned(),
            "Cmd" => "Ctrl".to_owned(),
            "Shift" if mac => "⇧".to_owned(),
            "Alt" if mac => "⌥".to_owned(),
            "-" => "−".to_owned(),
            key => key.to_owned(),
        }
    };
    let mut right = right_center.x;
    let alternatives: Vec<&str> = keys.split(", ").collect();
    for (alternative, chords) in alternatives.into_iter().rev().enumerate() {
        if alternative > 0 {
            let slash = painter.layout_no_wrap("/".to_owned(), theme::regular(11.0), ink);
            right -= 6.0 + slash.size().x;
            controls::galley_at(painter, pos2(right, right_center.y), slash);
            right -= 6.0;
        }
        for (step, chord) in held_over(chords).iter().rev().enumerate() {
            if step > 0 {
                right -= 8.0;
            }
            let joined = (chord.iter().copied())
                .map(cap)
                .collect::<Vec<_>>()
                .join("+");
            right -= controls::keycaps(painter, pos2(right, right_center.y), &joined, ink);
        }
    }
    right_center.x - right
}

/// The keys of each chord of one alternative, written apart by spaces. A
/// key written alone after a chord with modifiers is held with the same
/// ones, as the shorthand means it: "Shift+← →" is Shift+← and Shift+→,
/// and "Cmd+X C V" is Cmd+X, Cmd+C and Cmd+V. A bare → or C is another
/// shortcut, so a cap never stands without the keys held for it.
fn held_over(chords: &str) -> Vec<Vec<&str>> {
    let mut held: Vec<&str> = Vec::new();
    chords
        .split(' ')
        .filter(|chord| !chord.is_empty())
        .map(|chord| {
            let mut keys = controls::chord_keys(chord);
            if keys.len() > 1 {
                held = keys[..keys.len() - 1].to_vec();
            } else {
                keys.splice(0..0, held.iter().copied());
            }
            keys
        })
        .collect()
}

// ---- Work cards and toasts --------------------------------------------------

/// The width of a card over a viewer this wide.
fn card_width(viewer: Rect) -> f32 {
    (viewer.width() - 20.0).clamp(160.0, 360.0)
}

/// The surface of a toast and of a card of work under way: Terminal's
/// notice, elevated and rounded, outlined and shadowed.
fn toast_surface(painter: &egui::Painter, rect: Rect, p: &Palette) {
    let radius = metrics::TOAST_RADIUS;
    painter.add(p.popup_shadow().as_shape(rect, radius));
    painter.rect_filled(rect, radius, p.elevated);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
}

/// The area a work card for `request` is drawn in, which the viewer reads
/// back to lay its capsules under the cards.
pub(crate) fn card_id(request: u64) -> Id {
    Id::new(("save-audio", request))
}

/// Files being written, each with its progress and Cancel, and written ones
/// offered to the project, each with Add to project and a way to dismiss
/// it: at the top of the viewer, in the shape of a toast. Hands back where
/// the cards are, for the toasts to keep clear of.
pub fn saves(
    ctx: &egui::Context,
    p: &Palette,
    cards: &[Card],
    viewer: Rect,
    actions: &mut Vec<Action>,
) -> Vec<Rect> {
    let mut y = viewer.top() + 10.0;
    let width = card_width(viewer);
    let mut shown_at = Vec::with_capacity(cards.len());
    for card in cards {
        let shown = egui::Area::new(card_id(card.request))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(viewer.center().x - width * 0.5, y))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(p.elevated)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(metrics::TOAST_RADIUS)
                    .shadow(p.popup_shadow())
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 8,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 22.0);
                        card_rows(ui, p, card, actions);
                    });
            });
        y = shown.response.rect.bottom() + 8.0;
        shown_at.push(shown.response.rect);
    }
    shown_at
}

/// One card's words and controls.
fn card_rows(ui: &mut Ui, p: &Palette, card: &Card, actions: &mut Vec<Action>) {
    ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
    ui.horizontal_top(|ui| {
        let (glyph, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        let (icon, tint) = match card.shown {
            Shown::Writing(_) => (Icon::Save, p.secondary),
            Shown::Written => (Icon::CircleCheck, p.green),
            Shown::Separating(_) => (Icon::Music, p.secondary),
        };
        icons::paint(ui.painter(), glyph, icon, tint);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&card.text)
                    .font(theme::regular(12.5))
                    .color(p.fg),
            )
            .wrap(),
        );
    });
    ui.spacing_mut().item_spacing.x = 8.0;
    // Work under way: how far it is, when it is known, and Cancel.
    let working = match card.shown {
        Shown::Writing(done) => Some((
            Some(done),
            "Stop saving; nothing is written",
            Action::CancelSaveAudio(card.request),
        )),
        Shown::Separating(done) => Some((
            done,
            "Stop separating; no stem is kept",
            Action::CancelSeparate(card.request),
        )),
        Shown::Written => None,
    };
    match working {
        Some((done, stop, cancelled)) => {
            // Not yet said: an empty bar and no figure, drawn once, so a
            // long silent run keeps the window asleep.
            let known = done.map(|done| done.clamp(0.0, 1.0));
            let figure = known.map_or_else(
                || "…".to_owned(),
                |done| format!("{} %", (done * 100.0).round() as i32),
            );
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let cancel = controls::button_sized(
                        ui,
                        *p,
                        "Cancel",
                        ButtonKind::Quiet,
                        ButtonSize::Compact,
                    )
                    .on_hover_text(stop);
                    if cancel.clicked() {
                        actions.push(cancelled);
                    }
                    ui.label(
                        egui::RichText::new(figure)
                            .font(theme::mono(11.0))
                            .color(p.secondary),
                    );
                    kit::progress_bar(ui, p, known, p.accent, &card.text);
                });
            });
        }
        None => {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let add = controls::button_sized(
                        ui,
                        *p,
                        "Add to project",
                        ButtonKind::Secondary,
                        ButtonSize::Compact,
                    )
                    .on_hover_text("Import the file into the library");
                    if add.clicked() {
                        actions.push(Action::AddSavedAudio(card.request));
                    }
                    let dismiss = controls::button_sized(
                        ui,
                        *p,
                        "Dismiss",
                        ButtonKind::Quiet,
                        ButtonSize::Compact,
                    );
                    if dismiss.clicked() {
                        actions.push(Action::DismissSavedAudio(card.request));
                    }
                });
            });
        }
    }
}

/// The most notices kept at once, which is as many as are shown.
const MOST_NOTICES: usize = 4;

/// Adds `notice` to those being shown. The same words twice are one notice
/// that stays the longer of the two, and a problem said again as
/// information is still a problem, which stays until it is dismissed. Past
/// four, the oldest information makes room first, never the notice just
/// told. Information never makes room by taking a problem away: with four
/// problems in sight, information is not shown at all, and a fifth problem
/// takes the place of the oldest, which has been in sight the longest.
pub fn tell(notices: &mut Vec<Notice>, notice: Notice) {
    if let Some(told) = (notices.iter_mut()).find(|told| told.text == notice.text) {
        told.problem |= notice.problem;
        told.until = if told.problem {
            f64::INFINITY
        } else {
            told.until.max(notice.until)
        };
        return;
    }
    notices.push(notice);
    if notices.len() > MOST_NOTICES {
        let told = notices.len() - 1;
        let oldest = (notices[..told].iter()).position(|notice| !notice.problem);
        let gone = match oldest {
            Some(oldest) => oldest,
            // Brief news does not compete with a failure still to be seen.
            None if !notices[told].problem => told,
            None => 0,
        };
        notices.remove(gone);
    }
}

/// Where the toasts whose clocks stand still are kept: those under the
/// pointer and those waiting for room, each with the time it had left.
fn paused_id() -> Id {
    Id::new("notice-paused")
}

fn paused(ctx: &egui::Context) -> Vec<(u64, f64)> {
    ctx.data(|data| data.get_temp(paused_id()))
        .unwrap_or_default()
}

/// Lets the notices whose time is up go, and hands back when the next one
/// that is not paused is up, if one is. A notice under the pointer or
/// waiting for room keeps the time it had left; a problem stays until it is
/// dismissed.
pub fn expire(ctx: &egui::Context, notices: &mut Vec<Notice>) -> Option<f64> {
    let now = ctx.input(|input| input.time);
    let paused = paused(ctx);
    let left = |notice: &Notice| {
        let id = notice.id();
        (paused.iter())
            .find(|(paused, _)| *paused == id)
            .map(|(_, left)| *left)
    };
    for notice in notices.iter_mut() {
        // A later deadline told since the clock stopped is kept.
        if let Some(left) = left(notice) {
            notice.until = notice.until.max(now + left);
        }
    }
    notices.retain(|notice| notice.until > now);
    (notices.iter())
        .filter(|notice| left(notice).is_none() && notice.until.is_finite())
        .map(|notice| notice.until)
        .min_by(f64::total_cmp)
}

/// Where the open sheets are: the one on top, and the one a question
/// asked over it lies on, which the toasts keep clear of as well.
fn open_sheets(ctx: &egui::Context) -> Vec<Rect> {
    ctx.memory(|memory| {
        let Some(top) = memory.top_modal_layer() else {
            return Vec::new();
        };
        let beneath = (ASKED_OVER_A_SHEET.iter())
            .filter(|(asked, _)| top.id == Id::new(*asked))
            .map(|(_, over)| egui::LayerId::new(egui::Order::Foreground, Id::new(*over)))
            .filter(|layer| memory.areas().is_visible(layer));
        std::iter::once(top)
            .chain(beneath)
            .filter_map(|layer| memory.area_rect(layer.id))
            .collect()
    })
}

/// Where the toast of `notice` was in the last frame, surface and all, if
/// it was shown.
#[cfg(test)]
fn toast_rect(ctx: &egui::Context, notice: &Notice) -> Option<Rect> {
    let id = Id::new(("notice", notice.id()));
    let layer = egui::LayerId::new(egui::Order::Foreground, id);
    ctx.memory(|memory| {
        (memory.areas().visible_last_frame(&layer))
            .then(|| memory.area_rect(id))
            .flatten()
    })
}

/// One toast, laid out: its words and its size.
struct Toast {
    galley: Arc<Galley>,
    size: Vec2,
}

/// Toasts at the window's top trailing corner under the toolbar, stacked
/// downward 8 apart, newest last, and pushed down past any of the `cards`
/// over the viewer they would cover. While a sheet is open (`over_sheet`, or
/// any modal egui knows of) they go to the bottom trailing corner and stack
/// upward, the newest nearest the edge, so that none is over the sheet;
/// those that do not fit wait, their clocks stopped, until there is room.
/// They are kept over the sheet's layer, so that a Dismiss in sight can be
/// pressed. A toast takes no input, so what is under it can still be
/// clicked; the pointer resting on it stops its clock all the same. A toast
/// is not an accessibility node; the Dismiss of a problem is.
pub fn notices(
    ctx: &egui::Context,
    p: &Palette,
    notices: &mut [Notice],
    cards: &[Rect],
    over_sheet: bool,
    actions: &mut Vec<Action>,
) {
    let mut paused = paused(ctx);
    if notices.is_empty() {
        if !paused.is_empty() {
            ctx.data_mut(|data| data.remove::<Vec<(u64, f64)>>(paused_id()));
        }
        return;
    }
    let now = ctx.input(|input| input.time);
    let screen = ctx.content_rect();
    let sheets = open_sheets(ctx);
    let covers = |rect: Rect| sheets.iter().any(|sheet| sheet.intersects(rect));
    let under = over_sheet || !sheets.is_empty();
    let widest = (screen.width() - 96.0).clamp(160.0, 440.0);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("notices"),
    ));
    let laid: Vec<Toast> = (notices.iter())
        .map(|notice| {
            // Mark, 10, words, and a problem's Dismiss after 10 more.
            let dismiss = if notice.problem {
                10.0 + 28.0 + 8.0
            } else {
                14.0
            };
            let mut job = egui::text::LayoutJob::simple(
                notice.text.clone(),
                theme::regular(12.5),
                p.fg,
                widest - 14.0 - 16.0 - 10.0 - dismiss,
            );
            job.wrap.max_rows = 3;
            let galley = painter.layout_job(job);
            let least = if notice.problem { 28.0 } else { 20.0 };
            let size = vec2(
                14.0 + 16.0 + 10.0 + galley.size().x + dismiss,
                galley.size().y.max(least) + 16.0,
            );
            Toast { galley, size }
        })
        .collect();

    // Where each is shown, by its place in `notices`; the rest wait.
    let right = screen.right() - 14.0;
    let mut placed: Vec<(usize, Rect)> = Vec::new();
    if under {
        let mut bottom = screen.bottom() - 14.0;
        for (index, toast) in laid.iter().enumerate().rev() {
            let at = |bottom: f32| {
                Rect::from_min_size(
                    pos2(right - toast.size.x, bottom - toast.size.y),
                    toast.size,
                )
            };
            let mut rect = at(bottom);
            // Beside a sheet that comes near the window's foot, the first
            // goes into the room left under it.
            if placed.is_empty() && covers(rect) {
                let lowest =
                    (sheets.iter()).fold(screen.top(), |lowest, sheet| lowest.max(sheet.bottom()));
                let gap = screen.bottom() - lowest - toast.size.y;
                rect = at(screen.bottom() - (gap * 0.5).clamp(0.0, 14.0));
            }
            if covers(rect) || rect.top() < screen.top() {
                break;
            }
            bottom = rect.top() - 8.0;
            placed.push((index, rect));
        }
    } else {
        let mut top = screen.top() + metrics::TOOLBAR_HEIGHT + 8.0;
        // At most four, the newest.
        let first = laid.len().saturating_sub(MOST_NOTICES);
        for (index, toast) in laid.iter().enumerate().skip(first) {
            let mut rect = Rect::from_min_size(pos2(right - toast.size.x, top), toast.size);
            // Never over a card of work, whose controls it would hide.
            while let Some(card) = (cards.iter()).find(|card| card.intersects(rect)) {
                rect = rect.translate(vec2(0.0, card.bottom() + 8.0 - rect.top()));
            }
            if rect.bottom() > screen.bottom() - 14.0 {
                break;
            }
            top = rect.bottom() + 8.0;
            placed.push((index, rect));
        }
    }

    let still = kit::reduced_motion(ctx);
    let mut stopped: Vec<u64> = Vec::new();
    for (index, notice) in notices.iter().enumerate() {
        let Some(&(_, rect)) = placed.iter().find(|(placed, _)| *placed == index) else {
            stopped.push(notice.id());
            continue;
        };
        let id = notice.id();
        let was_paused = paused.iter().any(|(paused, _)| *paused == id);
        // The last moments fade it out, unless its clock stands still.
        let opacity = if still || was_paused || !notice.until.is_finite() {
            1.0
        } else {
            ((notice.until - now) / f64::from(motion::TOAST)).clamp(0.0, 1.0) as f32
        };
        let galley = laid[index].galley.clone();
        let shown = egui::Area::new(Id::new(("notice", id)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                let (rect, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
                let painter = ui.painter();
                toast_surface(painter, rect, p);
                let text_top = rect.center().y - galley.size().y * 0.5;
                let first_row = galley.rows.first().map_or(16.0, |row| row.height());
                let (mark, ink) = if notice.problem {
                    (Icon::AlertCircle, p.red)
                } else {
                    (Icon::Info, p.secondary)
                };
                icons::paint(
                    painter,
                    Rect::from_center_size(
                        pos2(rect.left() + 14.0 + 8.0, text_top + first_row * 0.5),
                        Vec2::splat(16.0),
                    ),
                    mark,
                    ink,
                );
                painter.galley(pos2(rect.left() + 40.0, text_top), galley, p.fg);
            });
        // A sheet that opened after the toast went over it; it comes back up
        // over the sheet, which it never covers.
        if under {
            ctx.move_to_top(shown.response.layer_id);
        }
        // Only a problem's Dismiss takes the pointer, in its own layer over
        // the toast's.
        if notice.problem {
            let button = Rect::from_center_size(
                pos2(rect.right() - 8.0 - 14.0, rect.center().y),
                Vec2::splat(28.0),
            );
            let dismiss = egui::Area::new(Id::new(("notice-dismiss", id)))
                .order(egui::Order::Foreground)
                .fixed_pos(button.min)
                .constrain(false)
                .show(ctx, |ui| {
                    ui.multiply_opacity(opacity);
                    icons::button(ui, Icon::Close, "Dismiss").clicked()
                });
            if under {
                ctx.move_to_top(dismiss.response.layer_id);
            }
            if dismiss.inner {
                actions.push(Action::DismissNotice(id));
            }
        }
        // Resting on it holds it.
        if ctx
            .pointer_hover_pos()
            .is_some_and(|pointer| rect.contains(pointer))
        {
            stopped.push(id);
        }
    }

    // A clock that stops keeps the time it had left; one that runs again
    // goes on from there.
    let resumed = (paused.iter()).any(|(id, _)| !stopped.contains(id));
    paused.retain(|(id, _)| stopped.contains(id));
    for notice in notices.iter_mut() {
        let id = notice.id();
        if !stopped.contains(&id) {
            continue;
        }
        // A later deadline told meanwhile is kept.
        let left = notice.until - now;
        let left = match paused.iter_mut().find(|(paused, _)| *paused == id) {
            Some((_, kept)) => {
                *kept = kept.max(left);
                *kept
            }
            None => {
                paused.push((id, left));
                left
            }
        };
        notice.until = now + left;
    }
    ctx.data_mut(|data| data.insert_temp(paused_id(), paused));
    // The clocks are counted before the toasts are drawn, so one that runs
    // again needs another frame to be counted, and to be woken for.
    if resumed {
        ctx.request_repaint();
    }
}

/// Tints the window while files are held over it: a rounded outline inset
/// from the window's edge, and a capsule in its middle that says what a drop
/// does. A drop imports wherever it lands, so no region is singled out.
pub fn drop_target(ctx: &egui::Context, p: &Palette) {
    let screen = ctx.content_rect();
    let radius = super::window::corner_radius(ctx).saturating_sub(6);
    egui::Area::new(Id::new("drop-target"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(false)
        .show(ctx, |ui| {
            let (window, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            let painter = ui.painter();
            let outline = window.shrink(6.0);
            painter.rect_filled(outline, radius, theme::tint(p.accent, 0.08));
            painter.rect_stroke(
                outline,
                radius,
                Stroke::new(1.5, theme::tint(p.accent, 0.9)),
                StrokeKind::Inside,
            );
            let words = "Drop to import media";
            let text = painter.layout_no_wrap(words.to_owned(), theme::medium(12.5), p.fg);
            let capsule =
                Rect::from_center_size(window.center(), vec2(36.0 + text.size().x + 16.0, 36.0));
            controls::capsule(painter, capsule, *p);
            icons::paint(
                painter,
                Rect::from_center_size(
                    pos2(capsule.left() + 16.0 + 7.0, capsule.center().y),
                    Vec2::splat(14.0),
                ),
                Icon::Import,
                p.secondary,
            );
            controls::galley_at(
                painter,
                pos2(capsule.left() + 36.0, capsule.center().y),
                text,
            );
            let said = ui.interact(capsule, Id::new("drop-target-words"), Sense::hover());
            said.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, words));
        });
}

#[cfg(test)]
#[path = "settings_sheet_tests.rs"]
mod settings_sheet_tests;

#[cfg(test)]
#[path = "notices_tests.rs"]
mod notices_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, Appearance};
    use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2};

    /// The question a duck asks, drawn in real egui passes.
    struct Asked {
        ctx: egui::Context,
        palette: Palette,
        time: f64,
        /// The names accessibility was told of, with where they are.
        seen: Vec<(String, Rect)>,
    }

    impl Asked {
        fn new() -> Self {
            let ctx = egui::Context::default();
            let palette = Palette::new(Appearance::Dark, Accent::default());
            theme::install_fonts(&ctx);
            theme::apply(&ctx, &palette);
            ctx.enable_accesskit();
            Self {
                ctx,
                palette,
                time: 0.0,
                seen: Vec::new(),
            }
        }

        fn frame(&mut self, keyed: usize, events: Vec<Event>) -> Option<bool> {
            self.time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut answer = None;
            let (ctx, palette) = (self.ctx.clone(), self.palette);
            let mut output = ctx.run_ui(input, |ui| {
                answer = replace_keys(ui.ctx(), &palette, keyed);
            });
            output.textures_delta.clear();
            if let Some(update) = &output.platform_output.accesskit_update {
                self.seen = (update.nodes.iter())
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
            answer
        }

        /// Draws the question until it has settled where it is shown.
        fn open(&mut self, keyed: usize) {
            for _ in 0..4 {
                assert_eq!(self.frame(keyed, Vec::new()), None);
            }
        }

        fn at(&self, name: &str) -> Pos2 {
            (self.seen.iter())
                .find(|(seen, _)| seen == name)
                .map(|(_, rect)| rect.center())
                .unwrap_or_else(|| panic!("FIXTURE: nothing is named \"{name}\""))
        }

        /// Clicks `name`; hands back the answer of each frame.
        fn click(&mut self, keyed: usize, name: &str) -> Vec<Option<bool>> {
            let pos = self.at(name);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            vec![
                self.frame(keyed, vec![Event::PointerMoved(pos)]),
                self.frame(keyed, vec![button(true)]),
                self.frame(keyed, vec![button(false)]),
            ]
        }
    }

    /// The cards of files being saved, and a notice, drawn in real egui
    /// passes over a viewer `width` wide that reaches the window's trailing
    /// edge, where the toasts are.
    struct Cards {
        ctx: egui::Context,
        palette: Palette,
        time: f64,
        viewer: Rect,
        seen: Vec<(String, Option<f64>, Rect)>,
        actions: Vec<Action>,
        /// Where the cards were drawn, and the notice's toast, last frame.
        drawn: Vec<Rect>,
        notice: Option<Rect>,
    }

    impl Cards {
        fn new(width: f32) -> Self {
            let ctx = egui::Context::default();
            let palette = Palette::new(Appearance::Dark, Accent::default());
            theme::install_fonts(&ctx);
            theme::apply(&ctx, &palette);
            ctx.enable_accesskit();
            Self {
                ctx,
                palette,
                time: 0.0,
                viewer: Rect::from_min_size(pos2(1000.0 - width, 40.0), vec2(width, 400.0)),
                seen: Vec::new(),
                actions: Vec::new(),
                drawn: Vec::new(),
                notice: None,
            }
        }

        fn frame(&mut self, cards: &[Card], events: Vec<Event>) {
            self.time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (ctx, palette, viewer) = (self.ctx.clone(), self.palette, self.viewer);
            let mut shown = [Notice {
                text: "Imported 1 file".to_owned(),
                problem: false,
                until: f64::MAX,
            }];
            let mut actions = Vec::new();
            let mut drawn = Vec::new();
            let mut output = ctx.run_ui(input, |ui| {
                drawn = saves(ui.ctx(), &palette, cards, viewer, &mut actions);
                notices(ui.ctx(), &palette, &mut shown, &drawn, false, &mut actions);
            });
            self.actions.extend(actions);
            self.drawn = drawn;
            // The whole toast, not only its words: its surface reaches past
            // its mark and above and below them.
            self.notice = toast_rect(&self.ctx, &shown[0]);
            output.textures_delta.clear();
            if let Some(update) = &output.platform_output.accesskit_update {
                self.seen = (update.nodes.iter())
                    .filter_map(|(_, node)| {
                        let bounds = node.bounds()?;
                        let name = node.label().or_else(|| node.value())?.to_owned();
                        let rect = Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        );
                        Some((name, node.numeric_value(), rect))
                    })
                    .collect();
            }
        }

        fn at(&self, name: &str) -> Vec<Rect> {
            (self.seen.iter())
                .filter(|(seen, ..)| seen == name)
                .map(|(.., rect)| *rect)
                .collect()
        }

        fn click(&mut self, cards: &[Card], pos: Pos2) {
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(cards, vec![Event::PointerMoved(pos)]);
            self.frame(cards, vec![button(true)]);
            self.frame(cards, vec![button(false)]);
        }
    }

    #[test]
    fn a_file_being_saved_can_be_stopped_and_a_saved_one_added_or_dismissed() {
        let cards = [
            Card {
                request: 4,
                text: "Saving Answer as talk.wav".to_owned(),
                shown: Shown::Writing(0.42),
            },
            Card {
                request: 3,
                text: "Saved 3.0 s of Answer as answer.wav.".to_owned(),
                shown: Shown::Written,
            },
        ];
        for width in [900.0, 300.0] {
            let mut drawn = Cards::new(width);
            for _ in 0..3 {
                drawn.frame(&cards, Vec::new());
            }
            // The words, the progress by its value and each named control,
            // within the viewer.
            let bar = (drawn.seen.iter())
                .find(|(name, value, _)| name == "Saving Answer as talk.wav" && value.is_some())
                .expect("the progress is named");
            assert!((bar.1.unwrap() - 0.42).abs() < 1e-6);
            let named = ["Cancel", "Add to project", "Dismiss"].map(|name| {
                let rects = drawn.at(name);
                assert_eq!(rects.len(), 1, "{name} at {width}: {:?}", drawn.seen);
                rects[0]
            });
            for rect in named {
                assert!(drawn.viewer.contains_rect(rect), "{rect:?} in {width}");
            }
            assert!(
                named[0].bottom() < named[1].top(),
                "the writing card is first"
            );
            // §5.8: toasts left the viewer for the window's corner, and
            // keep clear of the cards there.
            let notice = drawn.at("Imported 1 file");
            assert!(
                notice.is_empty(),
                "a notice takes no input and is not a widget"
            );
            let notice = drawn.notice.expect("the notice is shown");
            assert_eq!(drawn.drawn.len(), 2, "both cards are handed back");
            for card in &drawn.drawn {
                assert!(
                    !card.intersects(notice),
                    "the notice at {notice:?} is not over the card at {card:?} in {width}"
                );
            }

            drawn.click(&cards, named[0].center());
            drawn.click(&cards, named[1].center());
            drawn.click(&cards, named[2].center());
            assert!(
                matches!(
                    drawn.actions.as_slice(),
                    [
                        Action::CancelSaveAudio(4),
                        Action::AddSavedAudio(3),
                        Action::DismissSavedAudio(3),
                    ]
                ),
                "{:?}",
                drawn.actions
            );
        }
    }

    #[test]
    fn a_separation_shows_how_far_it_is_once_said_and_can_be_stopped() {
        let cards = [
            Card {
                request: 6,
                text: "Separating the stems of Song".to_owned(),
                shown: Shown::Separating(None),
            },
            Card {
                request: 7,
                text: "Separating the stems of Talk".to_owned(),
                shown: Shown::Separating(Some(0.37)),
            },
        ];
        for width in [900.0, 300.0] {
            let mut drawn = Cards::new(width);
            for _ in 0..3 {
                drawn.frame(&cards, Vec::new());
            }
            // Not yet said: the bar is named without a value, and no figure
            // is made up. The bar is the named node four points high.
            let value = |name: &str| {
                (drawn.seen.iter())
                    .find(|(seen, _, rect)| seen == name && rect.height() < 5.0)
                    .map(|(_, value, _)| *value)
                    .expect("the progress is named")
            };
            assert_eq!(value("Separating the stems of Song"), None);
            assert!((value("Separating the stems of Talk").unwrap() - 0.37).abs() < 1e-6);
            assert!(!drawn.at("…").is_empty(), "{:?}", drawn.seen);
            assert!(!drawn.at("37 %").is_empty(), "{:?}", drawn.seen);
            let mut cancels = drawn.at("Cancel");
            cancels.sort_by(|a, b| a.top().total_cmp(&b.top()));
            cancels.dedup();
            assert_eq!(cancels.len(), 2, "{:?}", drawn.seen);
            for rect in &cancels {
                assert!(drawn.viewer.contains_rect(*rect), "{rect:?} in {width}");
            }
            drawn.click(&cards, cancels[0].center());
            assert!(
                matches!(drawn.actions.as_slice(), [Action::CancelSeparate(6)]),
                "{:?}",
                drawn.actions
            );
        }
    }

    #[test]
    fn the_duck_asks_before_volume_keys_are_replaced() {
        let mut asked = Asked::new();
        asked.open(1);
        assert!(
            asked
                .seen
                .iter()
                .any(|(name, _)| name == "Replace this clip's volume keyframes?")
        );
        assert!(asked.click(1, "Replace").contains(&Some(true)));

        let mut asked = Asked::new();
        asked.open(2);
        assert!(
            asked
                .seen
                .iter()
                .any(|(name, _)| name == "Replace the volume keyframes of 2 clips?")
        );
        // §5.9.5: the answer that replaces is at the trailing edge.
        assert!(asked.at("Replace").x > asked.at("Cancel").x);
        assert!(asked.click(2, "Cancel").contains(&Some(false)));

        // An Escape gives it up, as Cancel does.
        let mut asked = Asked::new();
        asked.open(1);
        let escape = Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(asked.frame(1, vec![escape]), Some(false));
    }

    /// Draws `ask` in real egui passes, one for each list of events, and
    /// hands back what it answered in each.
    fn answers<A>(
        mut ask: impl FnMut(&egui::Context, &Palette) -> Option<A>,
        frames: Vec<Vec<Event>>,
    ) -> Vec<Option<A>> {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        let mut said = Vec::new();
        for (at, events) in frames.into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(at as f64 * 0.1),
                events,
                ..Default::default()
            };
            let mut answer = None;
            let output = ctx.run_ui(input, |ui| answer = ask(ui.ctx(), &palette));
            output.drop_without_applying_deltas();
            said.push(answer);
        }
        said
    }

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// A key pressed and let go within one frame.
    fn struck(on: Key) -> Vec<Event> {
        vec![key(on, true), key(on, false)]
    }

    #[test]
    fn enter_confirms_only_a_primary_and_only_when_pressed_afresh() {
        // The Enter that opened the question, held down and repeating,
        // answers nothing; one pressed afresh gives the Primary.
        let unsaved = |ctx: &egui::Context, p: &Palette| unsaved(ctx, p, "Film", &Pending::Quit);
        let said = answers(
            unsaved,
            vec![
                vec![key(Key::Enter, true)],
                vec![],
                vec![key(Key::Enter, true)],
                vec![key(Key::Enter, false)],
                struck(Key::Enter),
            ],
        );
        assert_eq!(said, [None, None, None, None, Some(Answer::Save)]);

        // Where every answer but Cancel gives something up, Enter does
        // nothing, and Escape still says what it always did.
        let ducked = |ctx: &egui::Context, p: &Palette| replace_keys(ctx, p, 2);
        let said = answers(
            ducked,
            vec![
                vec![],
                vec![],
                struck(Key::Enter),
                vec![],
                struck(Key::Escape),
            ],
        );
        assert_eq!(said, [None, None, None, None, Some(false)]);
        let stopping = |ctx: &egui::Context, p: &Palette| stop_rendering(ctx, p, 1);
        let said = answers(
            stopping,
            vec![vec![], vec![], struck(Key::Enter), struck(Key::Escape)],
        );
        assert_eq!(said, [None, None, None, Some(false)]);

        // Recovery's default keeps the work; Escape decides nothing.
        let recovering = |ctx: &egui::Context, p: &Palette| recovery(ctx, p, true);
        let said = answers(recovering, vec![vec![], vec![], struck(Key::Enter)]);
        assert_eq!(said, [None, None, Some(Recovery::Restore)]);
        let said = answers(recovering, vec![vec![], vec![], struck(Key::Escape)]);
        assert_eq!(said, [None, None, Some(Recovery::Later)]);

        // A conflict never gives the edits up from the keyboard: its default
        // saves them elsewhere.
        let changed = Conflict {
            path: PathBuf::from("/home/someone/Film.nvp"),
            busy: false,
            open: true,
        };
        let conflicted = |ctx: &egui::Context, p: &Palette| conflict(ctx, p, "Film", &changed);
        let said = answers(conflicted, vec![vec![], vec![], struck(Key::Enter)]);
        assert_eq!(said, [None, None, Some(ConflictAnswer::SaveAs)]);
    }

    #[test]
    fn a_key_written_alone_after_a_chord_is_held_with_its_modifiers() {
        // §5.9.2, §7.3: every chord as it is, and a bare → or C is another
        // shortcut, so the shorthand's modifiers carry over.
        let caps = |chords| held_over(chords);
        assert_eq!(caps("Shift+← →"), [vec!["Shift", "←"], vec!["Shift", "→"]]);
        assert_eq!(caps("Alt+↑ ↓"), [vec!["Alt", "↑"], vec!["Alt", "↓"]]);
        assert_eq!(
            caps("Cmd+X C V"),
            [vec!["Cmd", "X"], vec!["Cmd", "C"], vec!["Cmd", "V"]]
        );
        assert_eq!(caps("Shift+Cmd+Z"), [vec!["Shift", "Cmd", "Z"]]);
        // With nothing held first, each key is its own shortcut.
        assert_eq!(caps("Home End"), [vec!["Home"], vec!["End"]]);
        assert_eq!(caps("J K L"), [vec!["J"], vec!["K"], vec!["L"]]);
        assert_eq!(caps("= -"), [vec!["="], vec!["-"]]);
        assert_eq!(
            caps("Cmd+= Cmd+- Cmd+0"),
            [vec!["Cmd", "="], vec!["Cmd", "-"], vec!["Cmd", "0"]]
        );
        // Every row of the pane comes apart into keys.
        for chord in &SHORTCUTS {
            for alternative in chord.keys().split(", ") {
                assert!(
                    held_over(alternative).iter().all(|keys| !keys.is_empty()),
                    "{}",
                    chord.what
                );
            }
        }
    }

    #[test]
    fn a_short_window_keeps_a_sheet_clear_of_the_title_bar() {
        // §5.9.1 in a window under about 600 tall: centred, the sheet's top
        // is under the toolbar, as Preferences' is.
        for window in [560.0, 600.0, 900.0] {
            let height = sheet_height(520.0, window);
            let top = (window - height) * 0.5;
            assert!(top >= metrics::TOOLBAR_HEIGHT + 3.0, "{window}: {top}");
        }
        assert_eq!(sheet_height(520.0, 900.0), 520.0);
        assert_eq!(sheet_height(520.0, 560.0), 464.0);
        // Too short for both, it is still a sheet to use.
        assert_eq!(sheet_height(520.0, 300.0), 240.0);
    }

    #[test]
    fn the_drop_target_is_rounded_and_says_what_a_drop_does() {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut shapes = Vec::new();
        let mut names = Vec::new();
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| drop_target(ui.ctx(), &palette));
            shapes = output
                .shapes
                .iter()
                .map(|clipped| clipped.shape.clone())
                .collect();
            if let Some(update) = &output.platform_output.accesskit_update {
                names = (update.nodes.iter())
                    .filter_map(|(_, node)| {
                        node.label().or_else(|| node.value()).map(str::to_owned)
                    })
                    .collect();
            }
            output.drop_without_applying_deltas();
        }
        let rects: Vec<_> = (shapes.iter())
            .filter_map(|shape| match shape {
                egui::Shape::Rect(rect) => Some(rect.clone()),
                _ => None,
            })
            .collect();
        let outline = (rects.iter())
            .find(|rect| rect.rect == screen.shrink(6.0) && rect.stroke.width > 0.0)
            .expect("an outline inset from the window's edge");
        assert_eq!(outline.corner_radius, egui::CornerRadius::same(10));
        assert!(
            !rects.iter().any(|rect| rect.rect == screen),
            "no square fill covers the window"
        );
        assert!(
            names.iter().any(|name| name == "Drop to import media"),
            "{names:?}"
        );
    }
}
