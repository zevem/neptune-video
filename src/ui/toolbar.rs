//! The toolbar: the project menu and name, history, and panel toggles.
use super::{
    Action, library, shortcut, transitions,
    widgets::{self, icon_button, icon_toggle, menu_item, menu_style},
};
use crate::{
    icons::{self, Icon},
    speed,
    theme::{self, Palette, metrics},
};
use editor_core::{ClipId, Project, Time};
use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Ui, Vec2, pos2, vec2};
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
    title(ui, rect, input, actions);
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
        item(ui, Icon::Volume, "Detach audio", "", detach(input));
        item(ui, Icon::Save, crate::extract::LABEL, "", save_audio(input));
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
fn title(ui: &mut Ui, rect: Rect, input: &ToolbarInput, actions: &mut Vec<Action>) {
    let p = input.p;
    ui.add_space(8.0);
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
    let room = (rect.width() - 430.0).max(60.0);
    let name = widgets::truncated(ui, input.name, theme::medium(13.0), p.fg, room.min(320.0));
    let (name_rect, _) =
        ui.allocate_exact_size(vec2(name.size().x, metrics::ICON_BUTTON), Sense::hover());
    ui.painter().galley(
        pos2(name_rect.left(), name_rect.center().y - name.size().y * 0.5),
        name,
        p.fg,
    );
    if !state.is_empty() && rect.width() > 620.0 {
        ui.add_space(8.0);
        let warned = input.conflict && !input.saving;
        let width = if warned { 104.0 } else { 60.0 };
        let (state_rect, response) =
            ui.allocate_exact_size(vec2(width, metrics::ICON_BUTTON), Sense::hover());
        ui.painter().text(
            state_rect.left_center(),
            Align2::LEFT_CENTER,
            state,
            theme::regular(12.0),
            if warned { p.yellow } else { p.muted },
        );
        if warned {
            response.on_hover_text(
                "This file was saved by something else after it was opened here. \
                 Your edits are kept in this window; Save asks what to do with them.",
            );
        }
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
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
    });
}

#[cfg(test)]
mod tests {
    use super::{Action, ToolbarInput, brief, detach, save_audio};
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
                inspector: true,
                project: &project,
                clips,
                playhead: Time::ZERO,
                freeze_hold: Time::ZERO,
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
                inspector: true,
                project: &project,
                clips,
                playhead: Time::ZERO,
                freeze_hold: Time::ZERO,
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
