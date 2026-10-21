//! The toolbar, which is also the window's title bar: the library's toggle,
//! the title that opens the project menu, the launcher of the agent's
//! command line, history, the render queue, Export and the inspector's
//! toggle.
use super::{
    Action, controls, kit, library, panel, render_queue, shortcut, transitions,
    widgets::{icon_button, menu_item, menu_separator, menu_style},
};
use crate::{
    export::queue::QueueSummary,
    icons::{self, Icon},
    speed,
    theme::{self, Palette, metrics},
};
use editor_core::{ClipId, Project, Time};
use eframe::egui::{self, Color32, Pos2, Rangef, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::path::PathBuf;

pub struct ToolbarInput<'a> {
    pub p: &'a Palette,
    pub name: &'a str,
    pub dirty: bool,
    /// Whether the project has a file yet.
    pub saved_once: bool,
    pub saving: bool,
    /// The last save was refused: the file changed on disk since it was opened.
    pub conflict: bool,
    pub undo: Option<&'a str>,
    pub redo: Option<&'a str>,
    pub recent: &'a [PathBuf],
    /// The library is shown, once it has settled.
    pub library: bool,
    /// How wide the library is where it shows, and how much of it is in
    /// view: 0 hidden, 1 shown, and between while it slides. Its toggle
    /// travels with its edge.
    pub library_width: f32,
    pub library_reveal: f32,
    /// The inspector and agent panel is shown, once it has settled.
    pub inspector: bool,
    /// The viewer's pane, from its leading to its trailing edge: the title
    /// lines up with its timecode and the launcher sits over its middle.
    pub viewer: Rangef,
    /// The viewer's pane where it comes to rest. What folds, and whether the
    /// launcher is a field, is chosen for it, so that neither changes
    /// midway through a panel's slide.
    pub settled: Rangef,
    /// A proposal of the agent's waits for review.
    pub agent_waiting: bool,
    /// What the menu's clip commands act on, captured when one is chosen:
    /// the selected clips and the playhead.
    pub project: &'a Project,
    pub clips: &'a [ClipId],
    pub playhead: Time,
    /// How long a frame frozen from the menu is held.
    pub freeze_hold: Time,
    /// The render queue at a glance.
    pub renders: QueueSummary,
}

/// Room kept between the title and whatever follows it.
const CLEAR: f32 = 14.0;
/// The least of the name kept while other controls can still give way, and
/// the least it is ever cut to.
const NAME_ROOMY: f32 = 120.0;
const NAME_LEAST: f32 = 60.0;
/// The title button's inside: before the name, and after it the chevron.
const TITLE_PAD: f32 = 8.0;
const CHEVRON: f32 = 10.0;
/// Between the title button and its save state.
const STATE_GAP: f32 = 8.0;
/// The dot that stands for the save state where its words do not fit.
const DOT: f32 = 6.0;
/// Between groups of the trailing controls, between undo and redo, and
/// between the queue and Export.
const GROUP: f32 = 8.0;
const PAIR: f32 = 2.0;
const QUEUE_GAP: f32 = 4.0;
/// The trailing controls' distance from the window's edge.
const EDGE: f32 = 8.0;

/// The keys that show and hide the library and the inspector: Terminal's.
/// On macOS ⌘O stays Open, so the inspector takes Option as well.
pub fn library_keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘B"
    } else {
        "Ctrl+Shift+B"
    }
}

pub fn inspector_keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌥⌘O"
    } else {
        "Ctrl+Shift+O"
    }
}

/// The menu's "Freeze frame": the selected clip under the playhead, or else
/// the top one there, as the F key has it.
fn freeze(input: &ToolbarInput) -> Action {
    let selected = match input.clips {
        [clip] => Some(*clip),
        _ => None,
    };
    let frozen = speed::freeze_target(input.project, selected, input.playhead)
        .ok_or_else(|| "Put the playhead on a video clip to freeze a frame.".to_owned())
        .and_then(|clip| {
            speed::freeze_command(input.project, clip, input.playhead, input.freeze_hold)
        });
    match frozen {
        Ok(command) => Action::Edit(command),
        Err(reason) => Action::Notify(reason),
    }
}

/// The menu's "Detach audio": the one selected clip, as the row is chosen.
/// A clip whose sound cannot be detached is told why when the edit is made.
fn detach(input: &ToolbarInput) -> Action {
    match input.clips {
        [clip] => Action::DetachAudio(*clip),
        _ => Action::Notify("Select one video clip to detach its audio.".to_owned()),
    }
}

/// The menu's "Save audio as…": the one selected clip's span of its sound,
/// as the row is chosen. A clip without sound is told why when it is asked.
fn save_audio(input: &ToolbarInput) -> Action {
    match input.clips {
        [clip] => Action::SaveAudio(crate::extract::Source::Clip(*clip)),
        _ => Action::Notify("Select one clip to save its audio.".to_owned()),
    }
}

/// A history label short enough for a tooltip. A step the agent planned is
/// named by its whole summary.
fn brief(label: &str) -> String {
    const LIMIT: usize = 60;
    let line = label.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= LIMIT && line.len() == label.trim().len() {
        return line.to_owned();
    }
    let cut: String = line.chars().take(LIMIT).collect();
    format!("{}…", cut.trim_end())
}

/// Where the toolbar folds what does not fit, step by step: the launcher to
/// its glyph, the name down to [`NAME_ROOMY`], undo and redo into More,
/// Export to its glyph, the save state to a dot and the name down to
/// [`NAME_LEAST`]. The toggles and the title button never fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fold {
    launcher: bool,
    /// The least the name may be cut to at this step; `None` keeps it whole.
    name: Option<u32>,
    more: bool,
    export: bool,
    dot: bool,
}

impl Fold {
    const LADDER: [Fold; 7] = [
        Fold {
            launcher: false,
            name: None,
            more: false,
            export: false,
            dot: false,
        },
        Fold {
            launcher: true,
            name: None,
            more: false,
            export: false,
            dot: false,
        },
        Fold {
            launcher: true,
            name: Some(NAME_ROOMY as u32),
            more: false,
            export: false,
            dot: false,
        },
        Fold {
            launcher: true,
            name: Some(NAME_ROOMY as u32),
            more: true,
            export: false,
            dot: false,
        },
        Fold {
            launcher: true,
            name: Some(NAME_ROOMY as u32),
            more: true,
            export: true,
            dot: false,
        },
        Fold {
            launcher: true,
            name: Some(NAME_ROOMY as u32),
            more: true,
            export: true,
            dot: true,
        },
        Fold {
            launcher: true,
            name: Some(NAME_LEAST as u32),
            more: true,
            export: true,
            dot: true,
        },
    ];
}

/// Where each trailing control goes for one step of the folds, laid out
/// from the window's edge in.
struct Trailing {
    inspector: Rect,
    export: Rect,
    queue: Option<Rect>,
    redo: Option<Rect>,
    undo: Option<Rect>,
    more: Option<Rect>,
    /// Import, and how much of its room it has: it fades in and makes its
    /// room as the library slides away.
    import: Option<(Rect, f32)>,
    launcher: Option<Rect>,
}

impl Trailing {
    fn left(&self) -> f32 {
        let import = self
            .import
            .map(|(rect, share)| rect.right() + GROUP - (GROUP + rect.width()) * share);
        [self.launcher, self.undo, self.more]
            .into_iter()
            .flatten()
            .map(|rect| rect.left())
            .chain(import)
            .fold(self.export.left(), f32::min)
    }
}

/// The trailing controls for `fold`, with `import` of Import's room: 0
/// while the library holds it, 1 while the library is away.
fn trailing(rect: Rect, input: &ToolbarInput, fold: Fold, export: f32, import: f32) -> Trailing {
    let middle = rect.center().y;
    // A control `width` wide ending at `right`.
    let cell = |right: f32, width: f32| {
        Rect::from_min_max(
            pos2(right - width, middle - 14.0),
            pos2(right, middle + 14.0),
        )
    };
    let mut right = rect.right() - EDGE;
    let inspector = cell(right, metrics::ICON_BUTTON);
    right = inspector.left() - GROUP;
    let mut out = Trailing {
        inspector,
        export: cell(
            right,
            if fold.export {
                metrics::ICON_BUTTON
            } else {
                export
            },
        ),
        queue: None,
        redo: None,
        undo: None,
        more: None,
        import: None,
        launcher: None,
    };
    right = out.export.left();
    if queued(&input.renders) {
        let queue = cell(right - QUEUE_GAP, metrics::ICON_BUTTON);
        out.queue = Some(queue);
        right = queue.left();
    }
    right -= GROUP;
    if fold.more {
        let more = cell(right, metrics::ICON_BUTTON);
        out.more = Some(more);
        right = more.left();
    } else {
        let redo = cell(right, metrics::ICON_BUTTON);
        let undo = cell(redo.left() - PAIR, metrics::ICON_BUTTON);
        (out.redo, out.undo) = (Some(redo), Some(undo));
        right = undo.left();
    }
    // The library holds Import; while it is away, Import is here.
    if import > 0.0 {
        out.import = Some((cell(right - GROUP, metrics::ICON_BUTTON), import));
        right -= (GROUP + metrics::ICON_BUTTON) * import;
    }
    if fold.launcher {
        out.launcher = Some(cell(right - GROUP, metrics::ICON_BUTTON));
    }
    out
}

/// Whether the queue has work: rendering, waiting its turn, or held for
/// the person to start.
fn queued(renders: &QueueSummary) -> bool {
    renders.active + renders.queued + renders.held > 0
}

/// How the save state is said.
struct State {
    words: &'static str,
    /// The file changed on disk since it was opened.
    warned: bool,
}

impl State {
    fn of(input: &ToolbarInput) -> Option<Self> {
        let words = if input.saving {
            "Saving…"
        } else if input.conflict {
            "Changed on disk"
        } else if input.dirty {
            "Edited"
        } else if input.saved_once {
            "Saved"
        } else {
            return None;
        };
        Some(Self {
            words,
            warned: input.conflict && !input.saving,
        })
    }
}

/// The toolbar across the top of the window, which is also its title bar:
/// the library's toggle, the title that opens the project menu, the
/// launcher of the agent's command line, history, the render queue, Export
/// and the inspector's toggle. Laid out first and then made in reading
/// order, so Tab walks it from the leading edge.
pub fn show(ui: &mut Ui, rect: Rect, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    let middle = rect.center().y;
    let painter = ui.painter().clone();

    // The library's toggle travels with the library's edge, between its
    // place beside the lights and the edge itself.
    let reveal = input.library_reveal.clamp(0.0, 1.0);
    let toggle_at =
        |reveal: f32| rect.left() + egui::lerp(94.0..=input.library_width - 22.0, reveal);
    let toggle = toggle_at(reveal);
    // The title's words start where the viewer's timecode does, clear of
    // the toggle.
    let text_from = |viewer: Rangef, toggle: f32| (viewer.min + 12.0).max(toggle + 14.0 + CLEAR);
    let text_left = text_from(input.viewer, toggle);

    let name_font = theme::medium(13.0);
    let whole = painter
        .layout_no_wrap(input.name.to_owned(), name_font.clone(), p.fg)
        .size()
        .x;
    let state = State::of(input);
    let state_font = theme::regular(12.0);
    let words = state.as_ref().map(|state| {
        let said = painter
            .layout_no_wrap(state.words.to_owned(), state_font.clone(), p.secondary)
            .size()
            .x;
        // A conflict is led by a dot in the attention colour.
        if state.warned { DOT + 4.0 + said } else { said }
    });
    let export_label = painter
        .layout_no_wrap("Export".to_owned(), theme::medium(13.0), p.fg)
        .size()
        .x;
    let export = 10.0 + metrics::ICON + 6.0 + export_label + 12.0;
    let launcher_width = (rect.width() * 0.32).clamp(210.0, 320.0);
    let launcher_over = |viewer: Rangef| {
        Rect::from_center_size(
            pos2(viewer.center(), middle),
            vec2(launcher_width, metrics::COMPACT_HEIGHT),
        )
    };
    let launcher = launcher_over(input.viewer);

    // The room the name has at a step of the folds, beside its state.
    let room = |fold: Fold, cluster: &Trailing, text_left: f32, launcher: Rect| {
        let limit = if fold.launcher {
            cluster.left()
        } else {
            launcher.left()
        } - CLEAR;
        let state = match words {
            Some(_) if fold.dot => STATE_GAP + DOT,
            Some(words) => STATE_GAP + words,
            None => 0.0,
        };
        limit - text_left - 4.0 - CHEVRON - TITLE_PAD - state
    };
    // The first step of the folds at which the name keeps what that step
    // allows it, where the panels come to rest.
    let fold = {
        let shown = if input.library { 1.0 } else { 0.0 };
        let text_left = text_from(input.settled, toggle_at(shown));
        let launcher = launcher_over(input.settled);
        Fold::LADDER
            .into_iter()
            .find(|fold| {
                let cluster = trailing(rect, input, *fold, export, 1.0 - shown);
                let launcher_fits = fold.launcher || launcher.right() + CLEAR <= cluster.left();
                let least = fold.name.map_or(whole, |least| whole.min(least as f32));
                launcher_fits && room(*fold, &cluster, text_left, launcher) >= least
            })
            .unwrap_or(Fold::LADDER[Fold::LADDER.len() - 1])
    };
    // Laid out where the panels are drawn in this frame.
    let cluster = trailing(rect, input, fold, export, 1.0 - reveal);
    let name_room = room(fold, &cluster, text_left, launcher);

    // The library's toggle. It is never drawn pressed: the library shows
    // whether it is open.
    let library = icon_at(
        ui,
        pos2(toggle.round(), middle),
        Icon::Sidebar,
        "Media library",
        library_keys(),
    );
    library.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            input.library,
            "Media library",
        )
    });
    if library.clicked() {
        actions.push(Action::ToggleLibrary);
    }

    let name_width = whole.min(name_room.max(0.0));
    let after_title = title(ui, input, pos2(text_left, middle), name_width, actions);
    if let Some(state) = state {
        save_state(ui, p, after_title + STATE_GAP, middle, &state, fold.dot);
    }

    if fold.launcher {
        if let Some(rect) = cluster.launcher {
            let response = at(ui, rect, "toolbar-launcher-icon", |ui| {
                icon_button(ui, p, Icon::Sparkles, &launcher_name())
            });
            if response.clicked() {
                actions.push(Action::OpenAgent { ask: false });
            }
        }
    } else if launch(ui, p, launcher) {
        actions.push(Action::OpenAgent { ask: false });
    }

    if let Some((rect, share)) = cluster.import {
        // As the library's Import, without the folder of the library's own.
        at(ui, rect, "toolbar-import", |ui| {
            ui.multiply_opacity(share);
            let import = icon_button(
                ui,
                p,
                Icon::Import,
                &format!("Import media… ({})", shortcut("Cmd+I")),
            );
            egui::Popup::menu(&import).show(|ui| {
                menu_style(ui, 220.0);
                let mut item = |ui: &mut Ui, icon, text: &str, keys: &str, action| {
                    if menu_item(ui, p, Some(icon), text, &shortcut(keys)).clicked() {
                        actions.push(action);
                        ui.close();
                    }
                };
                item(
                    ui,
                    Icon::File,
                    "Import files…",
                    "Cmd+I",
                    Action::ImportFiles,
                );
                item(
                    ui,
                    Icon::Folder,
                    "Import folder…",
                    "Shift+Cmd+I",
                    Action::ImportFolder,
                );
            });
        });
    }
    history(ui, input, &cluster, actions);

    if let Some(rect) = cluster.queue
        && at(ui, rect, "toolbar-queue", |ui| {
            queue_button(ui, p, &input.renders)
        })
        .clicked()
    {
        actions.push(Action::OpenRenderQueue);
    }
    let export = at(ui, cluster.export, "toolbar-export", |ui| {
        if fold.export {
            icons::button_with_hint(ui, Icon::Export, "Export", &shortcut("Cmd+E"))
        } else {
            labelled(ui, p, Icon::Export, "Export", &shortcut("Cmd+E"))
        }
    });
    if export.clicked() {
        actions.push(Action::OpenExport);
    }
    let inspector = at(ui, cluster.inspector, "toolbar-inspector", |ui| {
        panel::toggle(
            ui,
            *p,
            Icon::PanelRight,
            "Inspector and agent",
            inspector_keys(),
            input.inspector,
            input.agent_waiting && !input.inspector,
        )
    });
    if inspector.clicked() {
        actions.push(Action::ToggleInspector);
    }
}

/// The launcher's name: what it does, and the keys that do it too.
fn launcher_name() -> String {
    format!("Ask or type a command ({})", shortcut("Cmd+K"))
}

/// Makes a control in `rect`, whatever the toolbar's layout.
fn at<R>(ui: &mut Ui, rect: Rect, salt: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    controls::place(
        ui,
        rect,
        egui::Layout::left_to_right(egui::Align::Center),
        salt,
        add,
    )
}

/// An icon button placed by its centre, as Terminal's toolbar places them.
fn icon_at(ui: &mut Ui, center: Pos2, icon: Icon, label: &str, keys: &str) -> egui::Response {
    at(
        ui,
        Rect::from_center_size(center, Vec2::splat(metrics::ICON_BUTTON)),
        label,
        |ui| icons::button_with_hint(ui, icon, label, keys),
    )
}

/// The title button: the project's name and a chevron, opening the project
/// menu. Its words start at `text`, the name cut to `name_width`. Returns
/// where the button ends.
fn title(
    ui: &mut Ui,
    input: &ToolbarInput,
    text: Pos2,
    name_width: f32,
    actions: &mut Vec<Action>,
) -> f32 {
    let p = input.p;
    let (text_left, middle) = (text.x, text.y);
    let name = controls::elided(
        ui.painter(),
        input.name,
        theme::medium(13.0),
        p.fg,
        name_width,
    );
    let rect = Rect::from_min_max(
        pos2(text_left - TITLE_PAD, middle - 14.0),
        pos2(
            text_left + name.size().x + 4.0 + CHEVRON + TITLE_PAD,
            middle + 14.0,
        ),
    );
    let response = ui.interact(rect, ui.id().with("project-menu"), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Project menu"));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let painter = ui.painter();
    let radius = metrics::CONTROL_RADIUS;
    if open || response.is_pointer_button_down_on() {
        painter.rect_filled(rect, radius, p.pressed);
    } else {
        let hover = kit::hover_amount(ui, &response);
        if hover > 0.0 {
            painter.rect_filled(rect, radius, p.hover.gamma_multiply(hover));
        }
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, radius, *p);
    }
    let words = controls::galley_at(painter, pos2(text_left, middle), name);
    icons::paint(
        painter,
        Rect::from_center_size(
            pos2(words.right() + 4.0 + CHEVRON * 0.5, middle),
            Vec2::splat(CHEVRON),
        ),
        Icon::ChevronDown,
        p.muted,
    );
    // The name is said as well as painted.
    let name = ui.interact(words, ui.id().with("project-name"), Sense::hover());
    name.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, input.name));

    egui::Popup::menu(&response).show(|ui| {
        menu_style(ui, 260.0);
        // A short window scrolls the menu rather than cutting it off.
        let room = ui.ctx().content_rect().height() - metrics::TOOLBAR_HEIGHT - 16.0;
        let room = room.max(120.0);
        egui::ScrollArea::vertical()
            .max_height(room)
            .min_scrolled_height(room)
            .show(ui, |ui| menu(ui, input, actions));
    });
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.on_hover_text("Project menu");
    rect.right()
}

/// What "Changed on disk" means for the person's edits.
const CONFLICT_TIP: &str = "This file was saved by something else after it was opened here. \
    Your edits are kept in this window; Save asks what to do with them.";

/// Whether the project holds unsaved work, said beside its title: in words,
/// or as a dot named by them where they do not fit.
fn save_state(ui: &mut Ui, p: &Palette, left: f32, middle: f32, state: &State, dot: bool) {
    let tip = if state.warned {
        CONFLICT_TIP
    } else {
        state.words
    };
    // Changed on disk needs the person: its dot is in the attention colour,
    // and its words in the body ink, since the attention colour alone is
    // too faint to read in light.
    let attention = p.legible(p.attention);
    if dot {
        let rect = Rect::from_min_size(pos2(left, middle - 14.0), vec2(DOT, 28.0));
        let response = ui.interact(rect, ui.id().with("save-state"), Sense::hover());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, state.words));
        ui.painter().circle_filled(
            rect.center(),
            DOT * 0.5,
            if state.warned { attention } else { p.secondary },
        );
        response.on_hover_text(tip);
        return;
    }
    let mut x = left;
    if state.warned {
        ui.painter()
            .circle_filled(pos2(x + DOT * 0.5, middle), DOT * 0.5, attention);
        x += DOT + 4.0;
    }
    let ink = if state.warned { p.fg } else { p.secondary };
    let said = ui
        .painter()
        .layout_no_wrap(state.words.to_owned(), theme::regular(12.0), ink);
    // Regular 12 beside the title's Medium 13, on the title's baseline, as
    // Terminal sets its subtitle.
    let words = controls::galley_at(ui.painter(), pos2(x, middle + 0.5), said);
    let rect = Rect::from_min_max(
        pos2(left, middle - 14.0),
        pos2(words.right(), middle + 14.0),
    );
    let response = ui.interact(rect, ui.id().with("save-state"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, state.words));
    if state.warned {
        response.on_hover_text(tip);
    }
}

/// The launcher: a field-shaped button over the viewer that opens the
/// agent's command line, as Ctrl+K does. Terminal's command field.
fn launch(ui: &mut Ui, p: &Palette, field: Rect) -> bool {
    let middle = field.center().y;
    let response = ui.interact(field, ui.id().with("toolbar-launcher"), Sense::click());
    let name = launcher_name();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    let painter = ui.painter();
    painter.rect_filled(
        field,
        metrics::CONTROL_RADIUS,
        if response.is_pointer_button_down_on() {
            p.pressed
        } else if response.hovered() {
            p.hover
        } else {
            p.control
        },
    );
    if response.has_focus() {
        painter.rect_stroke(
            field,
            metrics::CONTROL_RADIUS,
            egui::Stroke::new(1.5, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    icons::paint(
        painter,
        Rect::from_center_size(pos2(field.left() + 16.0, middle), Vec2::splat(13.0)),
        Icon::Sparkles,
        p.muted,
    );
    let caps = controls::keycaps(
        painter,
        pos2(field.right() - 6.0, middle),
        &shortcut("Cmd+K"),
        p.muted,
    );
    controls::galley_at(
        painter,
        pos2(field.left() + 30.0, middle),
        controls::elided(
            painter,
            "Ask or type a command",
            theme::regular(12.5),
            p.muted,
            field.width() - 44.0 - caps,
        ),
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Undo and redo, or the More menu that holds them where they do not fit.
fn history(ui: &mut Ui, input: &ToolbarInput, cluster: &Trailing, actions: &mut Vec<Action>) {
    let p = input.p;
    let undo = match input.undo {
        Some(what) => (format!("Undo {}", brief(what)), true),
        None => ("Nothing to undo".to_owned(), false),
    };
    let redo = match input.redo {
        Some(what) => (format!("Redo {}", brief(what)), true),
        None => ("Nothing to redo".to_owned(), false),
    };
    let named = |(what, can): &(String, bool), keys: &str| match can {
        true => format!("{what} ({})", shortcut(keys)),
        false => what.clone(),
    };
    if let Some(rect) = cluster.undo {
        at(ui, rect, "toolbar-undo", |ui| {
            ui.add_enabled_ui(undo.1, |ui| {
                let response = icon_button(ui, p, Icon::Undo, &named(&undo, "Cmd+Z"))
                    .on_disabled_hover_text("Nothing to undo");
                if response.clicked() {
                    actions.push(Action::Undo);
                }
            });
        });
    }
    if let Some(rect) = cluster.redo {
        at(ui, rect, "toolbar-redo", |ui| {
            ui.add_enabled_ui(redo.1, |ui| {
                let response = icon_button(ui, p, Icon::Redo, &named(&redo, "Shift+Cmd+Z"))
                    .on_disabled_hover_text("Nothing to redo");
                if response.clicked() {
                    actions.push(Action::Redo);
                }
            });
        });
    }
    if let Some(rect) = cluster.more {
        at(ui, rect, "toolbar-more", |ui| {
            let surface = ui.painter().add(egui::Shape::Noop);
            let more = icon_button(ui, p, Icon::Dots, "More");
            if egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more)) {
                ui.painter().set(
                    surface,
                    egui::Shape::rect_filled(more.rect.shrink(1.0), 7, p.pressed),
                );
            }
            egui::Popup::menu(&more)
                .align(egui::RectAlign::BOTTOM_END)
                .show(|ui| {
                    menu_style(ui, 260.0);
                    for ((what, can), keys, action) in [
                        (&undo, "Cmd+Z", Action::Undo),
                        (&redo, "Shift+Cmd+Z", Action::Redo),
                    ] {
                        ui.add_enabled_ui(*can, |ui| {
                            let icon = match action {
                                Action::Undo => Icon::Undo,
                                _ => Icon::Redo,
                            };
                            let keys = if *can { shortcut(keys) } else { String::new() };
                            if menu_item(ui, p, Some(icon), what, &keys).clicked() {
                                actions.push(action);
                                ui.close();
                            }
                        });
                    }
                });
        });
    }
}

/// A quiet button with a glyph and a label, as Export is at the toolbar's
/// edge: no fill at rest, and never the accent, which is kept for the
/// button that confirms the export in its sheet.
fn labelled(ui: &mut Ui, p: &Palette, icon: Icon, text: &str, keys: &str) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::medium(13.0), p.fg);
    let size = vec2(
        10.0 + metrics::ICON + 6.0 + galley.size().x + 12.0,
        metrics::ICON_BUTTON,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = metrics::CONTROL_RADIUS;
        if response.is_pointer_button_down_on() {
            painter.rect_filled(rect, radius, p.pressed);
        } else {
            let hover = kit::hover_amount(ui, &response);
            if hover > 0.0 {
                painter.rect_filled(rect, radius, p.hover.gamma_multiply(hover));
            }
        }
        if response.has_focus() {
            controls::focus_ring(painter, rect, radius, *p);
        }
        let glyph = Rect::from_center_size(
            pos2(rect.left() + 10.0 + metrics::ICON * 0.5, rect.center().y),
            Vec2::splat(metrics::ICON),
        );
        icons::paint(painter, glyph, icon, p.fg);
        controls::galley_at(painter, pos2(glyph.right() + 6.0, rect.center().y), galley);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    icons::hint(response, text, keys)
}

/// The project menu's rows.
fn menu(ui: &mut Ui, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    let mut item = |ui: &mut Ui, icon: Icon, text: &str, keys: &str, action: Action| {
        if menu_item(ui, p, Some(icon), text, &shortcut(keys)).clicked() {
            actions.push(action);
            ui.close();
        }
    };
    item(ui, Icon::Plus, "New project", "Cmd+N", Action::NewProject);
    item(ui, Icon::Folder, "Open…", "Cmd+O", Action::OpenProject);
    controls::menu_submenu(ui, *p, Icon::History, "Open recent", |ui| {
        menu_style(ui, 240.0);
        if input.recent.is_empty() {
            ui.add_enabled_ui(false, |ui| {
                menu_item(ui, p, None, "No recent projects", "");
            });
        }
        for path in input.recent.iter().take(6) {
            let name = path.file_stem().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            item(ui, Icon::File, &name, "", Action::OpenRecent(path.clone()));
        }
    });
    menu_separator(ui, p);
    item(ui, Icon::Save, "Save", "Cmd+S", Action::Save);
    item(
        ui,
        Icon::FilePlus,
        "Save as…",
        "Shift+Cmd+S",
        Action::SaveAs,
    );
    item(ui, Icon::Copy, "Duplicate…", "", Action::DuplicateProject);
    item(
        ui,
        Icon::History,
        "Checkpoints…",
        "",
        Action::OpenCheckpoints,
    );
    menu_separator(ui, p);
    item(ui, Icon::Export, "Export…", "Cmd+E", Action::OpenExport);
    item(
        ui,
        Icon::List,
        "Render queue",
        "Shift+Cmd+E",
        Action::OpenRenderQueue,
    );
    menu_separator(ui, p);
    item(
        ui,
        Icon::Import,
        "Import media…",
        "Cmd+I",
        Action::ImportFiles,
    );
    item(
        ui,
        Icon::FolderPlus,
        "Import folder…",
        "Shift+Cmd+I",
        Action::ImportFolder,
    );
    menu_separator(ui, p);
    item(
        ui,
        Icon::Settings,
        "Preferences…",
        "Cmd+,",
        Action::OpenSettings,
    );
    menu_separator(ui, p);
    // The clips selected and the playhead as the row is chosen.
    controls::menu_submenu(ui, *p, Icon::Film, "Clip", |ui| {
        menu_style(ui, 240.0);
        item(
            ui,
            Icon::Transition,
            "Add transition",
            "Cmd+T",
            Action::AddTransitions {
                places: transitions::at_playhead(input.project, input.clips, input.playhead),
                kind: None,
            },
        );
        item(ui, Icon::Snowflake, "Freeze frame", "F", freeze(input));
        item(ui, Icon::Volume, "Detach audio", "", detach(input));
        item(ui, Icon::Save, crate::extract::LABEL, "", save_audio(input));
        item(ui, Icon::Diamond, "Keyframes", "U", Action::ToggleKeyLane);
        item(ui, Icon::Curve, "Graph editor", "G", Action::ToggleGraph);
        item(
            ui,
            Icon::Wand,
            "Effects, transitions and looks",
            "",
            Action::ShowLibrary(library::Tab::Effects),
        );
    });
}

/// What the queue button's tooltip says of the queue.
pub fn queue_state(renders: &QueueSummary) -> String {
    let keys = shortcut("Shift+Cmd+E");
    let mut said = Vec::new();
    if renders.active > 0 {
        let percent = (renders.progress.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0).floor();
        said.push(if renders.paused {
            format!("paused at {percent:.0} %")
        } else {
            format!("rendering, {percent:.0} %")
        });
    }
    if renders.queued > 0 {
        said.push(format!("{} waiting", renders.queued));
    }
    // Held jobs start only once started in the queue.
    if renders.held > 0 {
        said.push(format!("{} held", renders.held));
    }
    if said.is_empty() {
        format!("Render queue ({keys})")
    } else {
        format!("Render queue: {} ({keys})", said.join(" · "))
    }
}

/// The render queue's button, while the queue has work: a ring of the
/// rendering job's progress, with the count of jobs under way inside it when
/// there are several, or the list glyph while only held jobs wait for the
/// person to start them. Named "Render queue".
fn queue_button(ui: &mut Ui, p: &Palette, renders: &QueueSummary) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::ICON_BUTTON), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Render queue")
    });
    let busy = renders.active + renders.queued;
    if busy == 0 {
        kit::paint_icon_button(ui, p, &response, Icon::List, false, None);
    } else {
        // The surface and focus of an icon button, around the ring.
        kit::paint_icon_button(
            ui,
            p,
            &response,
            Icon::List,
            false,
            Some(Color32::TRANSPARENT),
        );
        let ring = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
        render_queue::summary_ring(ui, p, ring, renders.progress.unwrap_or(0.0), busy);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.on_hover_text(queue_state(renders))
}

#[cfg(test)]
#[path = "toolbar_tests.rs"]
mod toolbar_tests;

#[cfg(test)]
mod tests {
    use super::{Action, CONFLICT_TIP, ToolbarInput, brief, detach, save_audio};
    use crate::theme::{Accent, Appearance, Palette};
    use editor_core::{Project, Time};

    #[test]
    fn the_menu_detaches_the_audio_of_the_one_selected_clip() {
        let p = Palette::new(Appearance::default(), Accent::default());
        let project = Project::new("Menu");
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let chosen = |clips: &[editor_core::ClipId]| {
            detach(&ToolbarInput {
                p: &p,
                name: "Menu",
                dirty: false,
                saved_once: false,
                saving: false,
                conflict: false,
                undo: None,
                redo: None,
                recent: &[],
                library: true,
                library_width: 316.0,
                library_reveal: 1.0,
                inspector: true,
                viewer: eframe::egui::Rangef::new(316.0, 1140.0),
                settled: eframe::egui::Rangef::new(316.0, 1140.0),
                agent_waiting: false,
                project: &project,
                clips,
                playhead: Time::ZERO,
                freeze_hold: Time::ZERO,
                renders: crate::export::queue::QueueSummary::default(),
            })
        };
        // The clip is named when the row is chosen; whether its sound can be
        // detached is answered when the edit is made.
        assert!(matches!(chosen(&[first]), Action::DetachAudio(clip) if clip == first));
        for clips in [&[][..], &[first, second][..]] {
            let Action::Notify(notice) = chosen(clips) else {
                panic!("{} clips are not one", clips.len());
            };
            assert_eq!(notice, "Select one video clip to detach its audio.");
        }
    }

    #[test]
    fn the_menu_saves_the_audio_of_the_one_selected_clip() {
        let p = Palette::new(Appearance::default(), Accent::default());
        let project = Project::new("Menu");
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let chosen = |clips: &[editor_core::ClipId]| {
            save_audio(&ToolbarInput {
                p: &p,
                name: "Menu",
                dirty: false,
                saved_once: false,
                saving: false,
                conflict: false,
                undo: None,
                redo: None,
                recent: &[],
                library: true,
                library_width: 316.0,
                library_reveal: 1.0,
                inspector: true,
                viewer: eframe::egui::Rangef::new(316.0, 1140.0),
                settled: eframe::egui::Rangef::new(316.0, 1140.0),
                agent_waiting: false,
                project: &project,
                clips,
                playhead: Time::ZERO,
                freeze_hold: Time::ZERO,
                renders: crate::export::queue::QueueSummary::default(),
            })
        };
        // The clip is named when the row is chosen; whether it has sound to
        // save is answered when the dialog would be shown.
        assert!(matches!(
            chosen(&[first]),
            Action::SaveAudio(crate::extract::Source::Clip(clip)) if clip == first
        ));
        for clips in [&[][..], &[first, second][..]] {
            let Action::Notify(notice) = chosen(clips) else {
                panic!("{} clips are not one", clips.len());
            };
            assert_eq!(notice, "Select one clip to save its audio.");
        }
    }

    #[test]
    fn the_conflict_tooltip_reads_as_one_line() {
        assert_eq!(
            CONFLICT_TIP,
            "This file was saved by something else after it was opened here. Your edits are \
             kept in this window; Save asks what to do with them."
        );
        assert!(!CONFLICT_TIP.contains('\\') && !CONFLICT_TIP.contains("  "));
    }

    #[test]
    fn a_long_history_label_is_cut_for_its_tooltip() {
        assert_eq!(brief("Split clip"), "Split clip");
        let long = "Tighten the opening by trimming the first three clips and moving the title";
        let cut = brief(long);
        assert!(cut.ends_with('…') && cut.chars().count() <= 61, "{cut}");
        assert!(long.starts_with(cut.trim_end_matches('…')));
        // Only the first line of several is shown.
        assert_eq!(brief("Retitle\nand more"), "Retitle…");
    }
}
