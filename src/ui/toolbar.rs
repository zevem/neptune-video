//! The toolbar: the project menu and name, history, panel toggles, the
//! render queue and Export.
use super::{
    Action, library, render_queue, shortcut, transitions,
    widgets::{self, Kind, icon_button, icon_toggle, menu_item, menu_style},
};
use crate::{
    export::queue::QueueSummary,
    icons::{self, Icon},
    speed,
    theme::{self, Palette, metrics},
};
use editor_core::{ClipId, Project, Time};
use eframe::egui::{self, CornerRadius, Rect, Sense, Ui, Vec2, pos2, vec2};
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
    pub library: bool,
    pub inspector: bool,
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

/// The widest the project's name is drawn.
const NAME_MOST: f32 = 360.0;
/// The least of the name kept beside its save state.
const NAME_LEAST: f32 = 60.0;
/// Between the name and its state, and before the trailing controls.
const GAP: f32 = 8.0;
/// The dot that stands for the save state where its word does not fit.
const DOT: f32 = 6.0;

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

pub fn show(ui: &mut Ui, rect: Rect, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut bar;
    ui.spacing_mut().item_spacing.x = 2.0;

    // The application mark opens the project menu.
    let (mark, response) = ui.allocate_exact_size(vec2(46.0, metrics::ICON_BUTTON), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Project menu"));
    if response.hovered()
        || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response))
    {
        ui.painter()
            .rect_filled(mark, CornerRadius::same(metrics::CONTROL_RADIUS), p.hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            mark,
            CornerRadius::same(metrics::CONTROL_RADIUS),
            egui::Stroke::new(1.5, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    let tile = Rect::from_center_size(pos2(mark.left() + 16.0, mark.center().y), Vec2::splat(22.0));
    ui.painter()
        .rect_filled(tile, CornerRadius::same(7), p.accent);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(tile.center(), Vec2::splat(14.0)),
        Icon::Sparkles,
        p.on_accent,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(mark.right() - 9.0, mark.center().y), Vec2::splat(10.0)),
        Icon::ChevronDown,
        p.secondary,
    );
    egui::Popup::menu(&response).show(|ui| {
        menu_style(ui, 250.0);
        // A short window scrolls the menu rather than cutting it off.
        let room = ui.ctx().content_rect().height() - metrics::TOOLBAR_HEIGHT - 24.0;
        let room = room.max(120.0);
        let scroll = egui::ScrollArea::vertical()
            .max_height(room)
            .min_scrolled_height(room);
        scroll.show(ui, |ui| menu(ui, input, actions));
    });
    response.on_hover_text("Project menu");
    title(ui, input, actions);
}

/// The project menu's rows.
fn menu(ui: &mut Ui, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    {
        let mut item = |ui: &mut Ui, icon: Icon, text: &str, keys: &str, action: Action| {
            if menu_item(ui, p, Some(icon), text, &shortcut(keys)).clicked() {
                actions.push(action);
                ui.close();
            }
        };
        item(ui, Icon::Plus, "New project", "Cmd+N", Action::NewProject);
        item(ui, Icon::Folder, "Open…", "Cmd+O", Action::OpenProject);
        if !input.recent.is_empty() {
            ui.separator();
            ui.label(
                egui::RichText::new("Recent")
                    .font(theme::medium(11.0))
                    .color(p.muted),
            );
            for path in input.recent.iter().take(6) {
                let name = path.file_stem().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                item(ui, Icon::File, &name, "", Action::OpenRecent(path.clone()));
            }
        }
        ui.separator();
        item(ui, Icon::Save, "Save", "Cmd+S", Action::Save);
        item(ui, Icon::Save, "Save as…", "Shift+Cmd+S", Action::SaveAs);
        item(ui, Icon::Copy, "Duplicate…", "", Action::DuplicateProject);
        item(
            ui,
            Icon::History,
            "Checkpoints…",
            "",
            Action::OpenCheckpoints,
        );
        item(ui, Icon::Export, "Export…", "Cmd+E", Action::OpenExport);
        item(
            ui,
            Icon::List,
            "Render queue",
            "Shift+Cmd+E",
            Action::OpenRenderQueue,
        );
        ui.separator();
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
        ui.separator();
        item(
            ui,
            Icon::Settings,
            "Settings…",
            "Cmd+,",
            Action::OpenSettings,
        );
        ui.separator();
        ui.label(
            egui::RichText::new("Clip")
                .font(theme::medium(11.0))
                .color(p.muted),
        );
        // The clips selected and the playhead as the row is chosen.
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
        item(ui, Icon::Pause, "Freeze frame", "F", freeze(input));
        item(ui, Icon::Diamond, "Keyframes", "U", Action::ToggleKeyLane);
        item(ui, Icon::Curve, "Graph editor", "G", Action::ToggleGraph);
        item(
            ui,
            Icon::Sparkles,
            "Effects, transitions and looks",
            "",
            Action::ShowLibrary(library::Tab::Effects),
        );
    }
}

/// The project's name and state, and the controls at the trailing edge.
///
/// The trailing controls are laid out first, from the edge, and the name and
/// its state take what they leave: the state is shown in words while they
/// fit beside some of the name, and as a dot after it, with the words on
/// hover, where they do not.
fn title(ui: &mut Ui, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    ui.add_space(GAP);
    let start = ui.cursor().min.x;
    let row = ui.available_rect_before_wrap();
    let mut trailing = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    controls(&mut trailing, input, actions);
    let free = (trailing.min_rect().left() - GAP - start).max(0.0);

    // The project's name and whether it holds unsaved work.
    let state = if input.saving {
        "Saving…"
    } else if input.conflict {
        "Changed on disk"
    } else if input.dirty {
        "Edited"
    } else if input.saved_once {
        "Saved"
    } else {
        ""
    };
    let warned = input.conflict && !input.saving;
    let ink = if warned { p.yellow } else { p.muted };
    let said = (!state.is_empty())
        .then(|| (ui.painter()).layout_no_wrap(state.to_owned(), theme::regular(12.0), ink));
    let worded = said
        .as_ref()
        .is_some_and(|said| free >= NAME_LEAST + GAP + said.size().x);
    let room = match &said {
        Some(said) if worded => free - GAP - said.size().x,
        Some(_) => free - GAP - DOT,
        None => free,
    };
    let name = widgets::truncated(
        ui,
        input.name,
        theme::medium(13.0),
        p.fg,
        room.clamp(0.0, NAME_MOST),
    );
    let (name_rect, _) =
        ui.allocate_exact_size(vec2(name.size().x, metrics::ICON_BUTTON), Sense::hover());
    ui.painter().galley(
        pos2(name_rect.left(), name_rect.center().y - name.size().y * 0.5),
        name,
        p.fg,
    );
    let Some(said) = said else {
        return;
    };
    ui.add_space(GAP);
    let tip = if warned {
        "This file was saved by something else after it was opened here. Your edits are \
         kept in this window; Save asks what to do with them."
            .to_owned()
    } else {
        state.to_owned()
    };
    if worded {
        let (state_rect, response) =
            ui.allocate_exact_size(vec2(said.size().x, metrics::ICON_BUTTON), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, state));
        ui.painter().galley(
            pos2(
                state_rect.left(),
                state_rect.center().y - said.size().y * 0.5,
            ),
            said,
            ink,
        );
        if warned {
            response.on_hover_text(tip);
        }
    } else {
        // No room for the words: a dot, yellow for a conflict.
        let (dot, response) =
            ui.allocate_exact_size(vec2(DOT, metrics::ICON_BUTTON), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, state));
        ui.painter().circle_filled(
            dot.center(),
            DOT * 0.5,
            if warned { p.yellow } else { p.muted },
        );
        response.on_hover_text(tip);
    }
}

/// The controls at the trailing edge, from the edge in: Export, the render
/// queue, Settings, the panel toggles, redo and undo.
fn controls(ui: &mut Ui, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    ui.spacing_mut().item_spacing.x = 2.0;
    // Export is the one primary action of the window, at its edge.
    let export = widgets::button(ui, p, Some(Icon::Export), "Export", Kind::Primary)
        .on_hover_text(format!("Export ({})", shortcut("Cmd+E")));
    if export.clicked() {
        actions.push(Action::OpenExport);
    }
    if queue_button(ui, p, &input.renders).clicked() {
        actions.push(Action::OpenRenderQueue);
    }
    ui.add_space(6.0);
    if icon_button(
        ui,
        p,
        Icon::Settings,
        &format!("Settings ({})", shortcut("Cmd+,")),
    )
    .clicked()
    {
        actions.push(Action::OpenSettings);
    }
    ui.add_space(6.0);
    if icon_toggle(
        ui,
        p,
        Icon::PanelRight,
        input.inspector,
        "Inspector and agent",
    )
    .clicked()
    {
        actions.push(Action::ToggleInspector);
    }
    if icon_toggle(ui, p, Icon::Sidebar, input.library, "Media library").clicked() {
        actions.push(Action::ToggleLibrary);
    }
    ui.add_space(6.0);
    ui.add_enabled_ui(input.redo.is_some(), |ui| {
        let label = match input.redo {
            Some(what) => format!("Redo {} ({})", brief(what), shortcut("Shift+Cmd+Z")),
            None => "Nothing to redo".to_owned(),
        };
        if icon_button(ui, p, Icon::Redo, &label).clicked() {
            actions.push(Action::Redo);
        }
    });
    ui.add_enabled_ui(input.undo.is_some(), |ui| {
        let label = match input.undo {
            Some(what) => format!("Undo {} ({})", brief(what), shortcut("Cmd+Z")),
            None => "Nothing to undo".to_owned(),
        };
        if icon_button(ui, p, Icon::Undo, &label).clicked() {
            actions.push(Action::Undo);
        }
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

/// The render queue's button: the list glyph while nothing renders or waits
/// to (held jobs wait for the person), and a ring of the rendering job's
/// progress, with the count of jobs under way when there are several, while
/// something does. Named "Render queue".
fn queue_button(ui: &mut Ui, p: &Palette, renders: &QueueSummary) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(metrics::ICON_BUTTON), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Render queue")
    });
    let busy = renders.active + renders.queued;
    if busy == 0 {
        widgets::paint_icon_button(ui, p, rect, &response, Icon::List, None, false);
    } else if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
        if response.hovered() {
            ui.painter().rect_filled(rect, radius, p.hover);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.5, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        let ring = Rect::from_center_size(rect.center(), Vec2::splat(metrics::ICON + 2.0));
        render_queue::summary_ring(ui, p, ring, renders.progress.unwrap_or(0.0), busy);
    }
    response.on_hover_text(queue_state(renders))
}

#[cfg(test)]
#[path = "toolbar_tests.rs"]
mod toolbar_tests;

#[cfg(test)]
mod tests {
    use super::brief;

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
