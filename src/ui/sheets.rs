//! Sheets and messages that sit over the window: settings, the question
//! asked before unsaved work is replaced, and short notices.
use super::{
    Action, shortcut,
    widgets::{self, Kind, icon_button, row, section},
};
use crate::{
    ai::{Effort, Provider},
    icons::{self, Icon},
    proxies::Cache,
    settings::{PreviewQuality, PreviewSettings, ProxyMode, Scale, Settings},
    theme::{self, Accent, Appearance, Palette, metrics},
};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::path::PathBuf;

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
    /// Seconds since the application started at which it goes away.
    pub until: f64,
}

pub(crate) fn sheet(p: &Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(p.elevated)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(metrics::SHEET_RADIUS))
        .shadow(p.sheet_shadow())
        .inner_margin(egui::Margin::same(18))
}

/// An on/off control drawn as a switch.
fn switch(ui: &mut Ui, p: &Palette, on: bool, label: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label));
    let t = ui.ctx().animate_bool_with_time(response.id, on, 0.14);
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(11),
        theme::mix(
            if p.dark {
                theme::color(0x3a3a40)
            } else {
                theme::color(0xc9c9cf)
            },
            p.accent,
            t,
        ),
    );
    let knob = pos2(rect.left() + 11.0 + 14.0 * t, rect.center().y);
    ui.painter().circle_filled(knob, 9.0, Color32::WHITE);
    response.clicked()
}

/// The proxy cache sizes offered, in GiB.
const PROXY_BUDGETS: [u32; 7] = [5, 10, 20, 50, 100, 200, 500];

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

/// A line of muted text under a control, wrapped.
fn note(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(11.5))
                .color(p.muted),
        )
        .wrap(),
    );
}

/// The rows of the settings sheet's Preview section.
fn preview(
    ui: &mut Ui,
    p: &Palette,
    preview: &PreviewSettings,
    cache: Cache,
    actions: &mut Vec<Action>,
) {
    row(ui, p, "Quality", |ui| {
        let mut quality = preview.quality;
        let options = PreviewQuality::ALL.map(|quality| (quality, quality.name()));
        if widgets::segmented(ui, p, &mut quality, &options, 236.0) {
            actions.push(Action::SetPreviewQuality(quality));
        }
    });
    note(ui, p, quality_note(preview.quality));
    row(ui, p, "Proxies", |ui| {
        let mut mode = preview.proxies;
        let options = ProxyMode::ALL.map(|mode| (mode, mode.name()));
        if widgets::segmented(ui, p, &mut mode, &options, 180.0) {
            actions.push(Action::SetProxyMode(mode));
        }
    });
    note(
        ui,
        p,
        "Proxies are smaller copies used only for preview; exports always use the \
         original media.",
    );
    if !cache.available {
        note(ui, p, "Proxies are unavailable on this computer.");
        return;
    }
    row(ui, p, "Cache size", |ui| {
        let chosen = preview.proxy_budget_gib;
        let mut pick = chosen;
        egui::ComboBox::from_id_salt("proxy-budget")
            .icon(widgets::combo_icon)
            .selected_text(format!("{chosen} GB"))
            .width(96.0)
            .show_ui(ui, |ui| {
                // A size kept from elsewhere stays offered.
                if !PROXY_BUDGETS.contains(&chosen) {
                    ui.selectable_value(&mut pick, chosen, format!("{chosen} GB"));
                }
                for gib in PROXY_BUDGETS {
                    ui.selectable_value(&mut pick, gib, format!("{gib} GB"));
                }
            })
            .response
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Proxy cache size")
            });
        if pick != chosen {
            actions.push(Action::SetProxyBudget(pick));
        }
    });
    let usage = cache_usage(cache.bytes, preview.proxy_budget_gib);
    row(ui, p, &usage, |ui| {
        let empty = cache.proxies == 0;
        let clear = ui
            .add_enabled_ui(!empty, |ui| {
                widgets::button(ui, p, Some(Icon::Trash), "Clear proxies", Kind::Plain)
            })
            .inner
            .on_hover_text(
                "Remove every proxy. None is made again until you ask for one or reopen the \
                 project.",
            )
            .on_disabled_hover_text("There are no proxies to clear");
        if clear.clicked() {
            actions.push(Action::ClearProxies);
        }
    });
}

/// The settings sheet. `cache` is what the proxy cache holds now. Returns
/// whether it should close.
pub fn settings(
    ctx: &egui::Context,
    p: &Palette,
    settings: &Settings,
    cache: Cache,
    actions: &mut Vec<Action>,
) -> bool {
    let mut close = false;
    let response = egui::Modal::new(Id::new("settings-sheet"))
        .backdrop_color(p.scrim)
        .frame(sheet(p))
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect();
            ui.set_width(400.0f32.min(screen.width() - 64.0));
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Settings")
                        .font(theme::semibold(15.0))
                        .color(p.fg),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, p, Icon::Close, "Close settings").clicked() {
                        close = true;
                    }
                });
            });

            // One scroll for the whole sheet: at a short window every section
            // stays reachable.
            let room = (screen.height() - 150.0).max(160.0);
            egui::ScrollArea::vertical()
                .max_height(room)
                // A sheet has no height of its own to offer: without this
                // the list scrolls long before the window is full.
                .min_scrolled_height(room)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    section(ui, p, "Appearance");
                    group(ui, p, |ui| {
                        row(ui, p, "Theme", |ui| {
                            let mut appearance = settings.appearance;
                            if widgets::segmented(
                                ui,
                                p,
                                &mut appearance,
                                &[(Appearance::Dark, "Dark"), (Appearance::Light, "Light")],
                                150.0,
                            ) {
                                actions.push(Action::SetAppearance(appearance));
                            }
                        });
                        row(ui, p, "Accent", |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (accent, name) in Accent::ALL.into_iter().rev() {
                                let (rect, response) =
                                    ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                                let selected = settings.accent == accent;
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::RadioButton,
                                        true,
                                        selected,
                                        name,
                                    )
                                });
                                let color = accent.color(p.dark);
                                ui.painter().circle_filled(rect.center(), 8.0, color);
                                if selected {
                                    ui.painter().circle_stroke(
                                        rect.center(),
                                        10.5,
                                        Stroke::new(1.5, color),
                                    );
                                }
                                if response.on_hover_text(name).clicked() {
                                    actions.push(Action::SetAccent(accent));
                                }
                            }
                        });
                        row(ui, p, "Interface size", |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            // The scale as shown, which the window may hold
                            // below the one asked for.
                            let room = super::largest_scale(ui.ctx());
                            let scale = settings.interface_scale.min(room);
                            // A step that would change nothing is not offered.
                            let step = |ui: &mut Ui, step: Scale, icon, label: &str| {
                                let to = step.applied_to(scale).min(room);
                                let label = shortcut(label);
                                let button = ui
                                    .add_enabled_ui(to != scale, |ui| {
                                        icon_button(ui, p, icon, &label)
                                    })
                                    .inner;
                                button.clicked().then_some(to)
                            };
                            let larger = step(ui, Scale::Larger, Icon::ZoomIn, "Larger (Cmd+=)");
                            // The size itself, which goes back to 100%.
                            let size = format!("{:.0}%", scale * 100.0);
                            let actual = ui
                                .add_enabled_ui(scale != 1.0, |ui| {
                                    ui.add_sized(
                                        [48.0, metrics::CONTROL_HEIGHT],
                                        egui::Button::new(
                                            egui::RichText::new(size)
                                                .font(theme::regular(12.0))
                                                .color(p.fg),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text(shortcut("Actual size (Cmd+0)"))
                                })
                                .inner
                                .clicked()
                                .then(|| Scale::Actual.applied_to(scale));
                            let smaller =
                                step(ui, Scale::Smaller, Icon::ZoomOut, "Smaller (Cmd+-)");
                            if let Some(to) = larger.or(actual).or(smaller) {
                                actions.push(Action::ScaleInterface(to));
                            }
                        });
                    });

                    section(ui, p, "Agent");
                    group(ui, p, |ui| {
                        row(ui, p, "Answered by", |ui| {
                            let mut provider = settings.ai.provider;
                            if widgets::segmented(
                                ui,
                                p,
                                &mut provider,
                                &[(Provider::Claude, "Claude"), (Provider::Codex, "Codex")],
                                150.0,
                            ) {
                                actions.push(Action::SetAiProvider(provider));
                            }
                        });
                        row(ui, p, "Model", |ui| {
                            let mut model = settings.ai.model().to_owned();
                            let field = egui::Frame::NONE
                                .fill(p.pressed)
                                .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS - 2))
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut model)
                                            .hint_text("Provider's default")
                                            .frame(egui::Frame::NONE)
                                            .desired_width(150.0)
                                            .font(theme::regular(12.5)),
                                    )
                                })
                                .inner;
                            if field.changed() {
                                actions.push(Action::SetAiModel(model));
                            }
                        });
                        row(ui, p, "Effort", |ui| {
                            let mut effort = settings.ai.effort;
                            let options = Effort::ALL.map(|effort| (effort, effort.name()));
                            if widgets::segmented(ui, p, &mut effort, &options, 236.0) {
                                actions.push(Action::SetAiEffort(effort));
                            }
                        });
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Requests go through the Claude or Codex command line \
                                 installed on this computer, with the sign-in it already \
                                 has. Aster keeps no key or password.",
                                )
                                .font(theme::regular(11.5))
                                .color(p.muted),
                            )
                            .wrap(),
                        );
                    });

                    section(ui, p, "Editing");
                    group(ui, p, |ui| {
                        row(ui, p, "Snap while dragging", |ui| {
                            if switch(ui, p, settings.snapping, "Snap while dragging") {
                                actions.push(Action::ToggleSnapping);
                            }
                        });
                        row(ui, p, "Keep a recovery copy while editing", |ui| {
                            if switch(ui, p, settings.autosave, "Keep a recovery copy") {
                                actions.push(Action::SetAutosave(!settings.autosave));
                            }
                        });
                    });

                    section(ui, p, "Preview");
                    group(ui, p, |ui| {
                        preview(ui, p, &settings.preview, cache, actions)
                    });

                    section(ui, p, "Keyboard");
                    group(ui, p, |ui| {
                        for (what, keys) in SHORTCUTS {
                            row(ui, p, what, |ui| {
                                ui.label(
                                    egui::RichText::new(shortcut(keys))
                                        .font(theme::regular(12.0))
                                        .color(p.fg),
                                );
                            });
                        }
                    });
                });
        });
    close || response.should_close()
}

const SHORTCUTS: [(&str, &str); 33] = [
    ("Play or pause", "Space"),
    ("Previous or next frame", "← →"),
    ("Back or forward one second", "Shift+← →"),
    ("Start or end", "Home End"),
    ("Back one second, stop, play faster", "J K L"),
    ("Select the clip under the playhead", "C"),
    ("Select the clip before or after", "Alt+← →"),
    ("Select the clip on the track above or below", "Alt+↑ ↓"),
    ("Split at playhead", "S"),
    ("Delete", "Delete"),
    ("Ripple delete", "Shift+Delete"),
    ("Undo", "Cmd+Z"),
    ("Redo", "Shift+Cmd+Z"),
    ("Cut, copy, paste", "Cmd+X C V"),
    ("Duplicate", "Cmd+D"),
    ("Select all", "Cmd+A"),
    ("Group, ungroup", "Cmd+G, Shift+Cmd+G"),
    ("Add marker", "M"),
    ("Add text", "T"),
    ("Add transition", "Cmd+T"),
    ("Freeze frame", "F"),
    ("Keyframes of the clip", "U"),
    ("Graph editor", "G"),
    ("Snapping", "N"),
    ("Zoom timeline", "Cmd+Scroll, = -"),
    ("Fit timeline", "Shift+Z"),
    ("Ask the agent", "Shift+Cmd+K"),
    ("Command line", "Cmd+K"),
    ("Save", "Cmd+S"),
    ("Import media", "Cmd+I"),
    ("Export", "Cmd+E"),
    ("Render queue", "Shift+Cmd+E"),
    ("Interface size", "Cmd+= Cmd+- Cmd+0"),
];

pub(crate) fn group(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .fill(p.control)
        .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS + 2))
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            add(ui);
        });
}

/// Asks what to do with unsaved work before it is replaced.
pub fn unsaved(ctx: &egui::Context, p: &Palette, name: &str, pending: &Pending) -> Option<Answer> {
    let mut answer = None;
    let response = egui::Modal::new(Id::new("unsaved-sheet"))
        .backdrop_color(p.scrim)
        .frame(sheet(p))
        .show(ctx, |ui| {
            ui.set_width(360.0f32.min(ui.ctx().content_rect().width() - 64.0));
            ui.label(
                egui::RichText::new(format!("Save changes to “{name}”?"))
                    .font(theme::semibold(15.0))
                    .color(p.fg),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(match pending {
                    Pending::Quit => "Your changes are lost if you quit without saving.",
                    _ => "Your changes are lost if you continue without saving.",
                })
                .font(theme::regular(12.5))
                .color(p.secondary),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, p, None, "Don't save", Kind::Destructive).clicked() {
                    answer = Some(Answer::Discard);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, p, None, "Save", Kind::Primary).clicked() {
                        answer = Some(Answer::Save);
                    }
                    if widgets::button(ui, p, None, "Cancel", Kind::Plain).clicked() {
                        answer = Some(Answer::Cancel);
                    }
                });
            });
        });
    if answer.is_none() && response.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

/// How a short question was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    /// The primary button, on the right.
    Primary,
    /// The button on the left that gives something up.
    GiveUp,
    /// The plain button, Escape or the backdrop.
    Dismissed,
}

/// A short question with a primary answer on the right, an optional plain
/// one beside it and an optional one on the left that gives something up.
#[expect(
    clippy::too_many_arguments,
    reason = "each question names its own words; a struct would only rename them"
)]
fn question(
    ctx: &egui::Context,
    p: &Palette,
    id: &str,
    title: &str,
    body: &str,
    give_up: Option<&str>,
    primary: &str,
    plain: Option<&str>,
) -> Option<Choice> {
    let mut answer = None;
    let response = egui::Modal::new(Id::new(id))
        .backdrop_color(p.scrim)
        .frame(sheet(p))
        .show(ctx, |ui| {
            ui.set_width(380.0f32.min(ui.ctx().content_rect().width() - 64.0));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(title)
                        .font(theme::semibold(15.0))
                        .color(p.fg),
                )
                .wrap(),
            );
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(body)
                        .font(theme::regular(12.5))
                        .color(p.secondary),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if let Some(text) = give_up
                    && widgets::button(ui, p, None, text, Kind::Destructive).clicked()
                {
                    answer = Some(Choice::GiveUp);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, p, None, primary, Kind::Primary).clicked() {
                        answer = Some(Choice::Primary);
                    }
                    if let Some(text) = plain
                        && widgets::button(ui, p, None, text, Kind::Plain).clicked()
                    {
                        answer = Some(Choice::Dismissed);
                    }
                });
            });
        });
    if answer.is_none() && response.should_close() {
        answer = Some(Choice::Dismissed);
    }
    answer
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
    let (title, body) = match also {
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
    let choice = question(
        ctx,
        p,
        "replace-export-sheet",
        &title,
        &body,
        None,
        "Replace",
        Some("Cancel"),
    );
    choice.map(|choice| choice == Choice::Primary)
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
    let choice = question(
        ctx,
        p,
        "stop-rendering-sheet",
        "Rendering will stop",
        &body,
        Some("Stop and quit"),
        "Keep rendering",
        None,
    );
    choice.map(|choice| choice == Choice::GiveUp)
}

/// Says that a save was refused for another writer's sake, and asks what to
/// do with the edits, which are still in the window. Nothing here writes the
/// file over what the other writer left.
pub fn conflict(
    ctx: &egui::Context,
    p: &Palette,
    name: &str,
    conflict: &Conflict,
) -> Option<ConflictAnswer> {
    let mut answer = None;
    let file = conflict.path.file_name().map_or_else(
        || conflict.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let response = egui::Modal::new(Id::new("conflict-sheet"))
        .backdrop_color(p.scrim)
        .frame(sheet(p))
        .show(ctx, |ui| {
            ui.set_width(400.0f32.min(ui.ctx().content_rect().width() - 64.0));
            ui.horizontal(|ui| {
                let (glyph, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                icons::paint(ui.painter(), glyph, Icon::Warning, p.yellow);
                ui.label(
                    egui::RichText::new(if conflict.busy {
                        format!("“{file}” is being saved elsewhere")
                    } else {
                        format!("“{file}” changed on disk")
                    })
                    .font(theme::semibold(15.0))
                    .color(p.fg),
                );
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(if conflict.busy {
                    format!(
                        "Another editor is writing this file right now, so “{name}” was \
                         not saved. Your edits are still here. Try again in a moment, \
                         or save them to another file."
                    )
                } else if conflict.open {
                    format!(
                        "Something else saved this file after it was opened here, so \
                         “{name}” was not saved over it. Your edits are still here. Save \
                         them to another file, or give them up and reopen the file as \
                         it is now."
                    )
                } else {
                    format!(
                        "Something else wrote this file after it was chosen, so \
                         “{name}” was not saved over it. Your edits are still here. \
                         Save them to another file."
                    )
                })
                .font(theme::regular(12.5))
                .color(p.secondary),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(conflict.path.display().to_string())
                    .font(theme::mono(10.5))
                    .color(p.muted),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if conflict.open && !conflict.busy {
                    let reopen =
                        widgets::button(ui, p, None, "Discard and reopen", Kind::Destructive)
                            .on_hover_text(
                                "Give up the edits in this window and read the file again",
                            );
                    if reopen.clicked() {
                        answer = Some(ConflictAnswer::Reopen);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if conflict.busy {
                        if widgets::button(ui, p, None, "Try again", Kind::Primary).clicked() {
                            answer = Some(ConflictAnswer::Retry);
                        }
                        if widgets::button(ui, p, None, "Save as…", Kind::Plain).clicked() {
                            answer = Some(ConflictAnswer::SaveAs);
                        }
                    } else if widgets::button(ui, p, Some(Icon::Save), "Save as…", Kind::Primary)
                        .clicked()
                    {
                        answer = Some(ConflictAnswer::SaveAs);
                    }
                    if widgets::button(ui, p, None, "Not now", Kind::Plain).clicked() {
                        answer = Some(ConflictAnswer::Later);
                    }
                });
            });
        });
    if answer.is_none() && response.should_close() {
        answer = Some(ConflictAnswer::Later);
    }
    answer
}

/// Named checkpoints of the project: make one, or go back to one.
pub fn checkpoints(
    ctx: &egui::Context,
    p: &Palette,
    state: &mut CheckpointsState,
    has_file: bool,
) -> Option<CheckpointAnswer> {
    let mut answer = None;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let response = egui::Modal::new(Id::new("checkpoints-sheet"))
        .backdrop_color(p.scrim)
        .frame(sheet(p))
        .show(ctx, |ui| {
            ui.set_width(400.0f32.min(ui.ctx().content_rect().width() - 64.0));
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Checkpoints")
                        .font(theme::semibold(15.0))
                        .color(p.fg),
                );
                if state.busy {
                    ui.add(egui::Spinner::new().size(12.0).color(p.muted));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, p, Icon::Close, "Close checkpoints").clicked() {
                        answer = Some(CheckpointAnswer::Close);
                    }
                });
            });
            if !has_file {
                ui.label(
                    egui::RichText::new("Checkpoints are kept beside the project file.")
                        .font(theme::regular(12.5))
                        .color(p.secondary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, p, Some(Icon::Save), "Save project…", Kind::Primary)
                        .clicked()
                    {
                        answer = Some(CheckpointAnswer::SaveProject);
                    }
                });
                return;
            }

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let add = ui
                        .add_enabled_ui(!state.busy, |ui| {
                            widgets::button(ui, p, Some(Icon::Plus), "Save", Kind::Primary)
                        })
                        .inner
                        .on_hover_text("Save a checkpoint of the project as it is now");
                    let field = egui::Frame::NONE
                        .fill(p.control)
                        .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS))
                        .inner_margin(egui::Margin::symmetric(9, 5))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.name)
                                    .hint_text("Checkpoint name")
                                    .frame(egui::Frame::NONE)
                                    .desired_width(f32::INFINITY)
                                    .font(theme::regular(12.5)),
                            )
                        })
                        .inner;
                    if std::mem::take(&mut state.focus) {
                        field.request_focus();
                    }
                    let entered =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (add.clicked() || entered) && !state.busy {
                        let name = state.name.trim();
                        answer = Some(CheckpointAnswer::Create(if name.is_empty() {
                            "Checkpoint".to_owned()
                        } else {
                            name.to_owned()
                        }));
                        state.name.clear();
                    }
                });
            });

            match &state.list {
                None => {}
                Some(Err(problem)) => {
                    ui.label(
                        egui::RichText::new(problem)
                            .font(theme::regular(12.5))
                            .color(p.red),
                    );
                }
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(
                        egui::RichText::new("No checkpoints yet")
                            .font(theme::regular(12.5))
                            .color(p.muted),
                    );
                }
                Some(Ok(list)) => {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    egui::ScrollArea::vertical()
                        .max_height(264.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for checkpoint in list {
                                if checkpoint_row(ui, p, checkpoint, now, !state.busy) {
                                    answer = Some(CheckpointAnswer::Restore(checkpoint.clone()));
                                }
                            }
                        });
                }
            }
        });
    if answer.is_none() && response.should_close() {
        answer = Some(CheckpointAnswer::Close);
    }
    answer
}

/// One checkpoint: its name, its age and the way back to it.
fn checkpoint_row(
    ui: &mut Ui,
    p: &Palette,
    checkpoint: &Checkpoint,
    now: u64,
    ready: bool,
) -> bool {
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::CONTROL_HEIGHT + 6.0);
        let (glyph, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        icons::paint(ui.painter(), glyph, Icon::History, p.secondary);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let restore = ui
                .add_enabled_ui(ready, |ui| {
                    widgets::button(ui, p, None, "Restore", Kind::Plain)
                })
                .inner;
            restore.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ready,
                    format!("Restore {}", checkpoint.label),
                )
            });
            if let Some(made) = checkpoint.made {
                ui.label(
                    egui::RichText::new(crate::timefmt::age(now.saturating_sub(made)))
                        .font(theme::regular(11.5))
                        .color(p.muted),
                );
            }
            let room = ui.available_width() - 6.0;
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let name =
                    widgets::truncated(ui, &checkpoint.label, theme::medium(12.5), p.fg, room);
                let (rect, _) = ui.allocate_exact_size(
                    vec2(room.max(0.0), metrics::CONTROL_HEIGHT),
                    Sense::hover(),
                );
                ui.painter().galley(
                    pos2(rect.left(), rect.center().y - name.size().y * 0.5),
                    name,
                    p.fg,
                );
            });
            restore.clicked()
        })
        .inner
    })
    .inner
}

/// Notices at the top of the viewer, newest last: over the picture's
/// margin, where they cover no control. While a sheet is open
/// (`over_sheet`, or any modal egui knows of) they go to the bottom of the
/// window, under the sheet, so that they cover neither it nor its
/// controls: as many as fit there, the newest nearest the edge. They take
/// no input.
pub fn notices(
    ctx: &egui::Context,
    p: &Palette,
    notices: &[Notice],
    viewer: Rect,
    over_sheet: bool,
) {
    if notices.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("notices"),
    ));
    let now = ctx.input(|i| i.time);
    let sheet = ctx
        .memory(|memory| (memory.top_modal_layer()).and_then(|layer| memory.area_rect(layer.id)));
    let screen = ctx.content_rect();
    let under = over_sheet || sheet.is_some();
    let area = if under { screen } else { viewer };
    let width = (area.width() - 20.0).clamp(160.0, 360.0);
    let laid: Vec<_> = (notices.iter())
        .map(|notice| {
            let galley = painter.layout(
                notice.text.clone(),
                theme::regular(12.5),
                p.fg,
                width - 46.0,
            );
            let height = (galley.size().y + 18.0).max(36.0);
            (notice, galley, height)
        })
        .collect();
    let at = |top: f32, height: f32| {
        Rect::from_min_size(
            pos2(area.center().x - width * 0.5, top),
            vec2(width, height),
        )
    };
    let mut placed = Vec::with_capacity(laid.len());
    if under {
        // Up from the window's edge, newest first, while they stay under
        // the sheet; the newest is shown even where there is no room.
        let newest = laid.last().map_or(36.0, |(.., height)| *height);
        let floor = match sheet {
            Some(sheet) => ((screen.bottom() - sheet.bottom() - newest) * 0.5).clamp(0.0, 8.0),
            None => 8.0,
        };
        let ceiling = sheet.map_or(screen.top(), |sheet| sheet.bottom());
        let mut bottom = screen.bottom() - floor;
        for (notice, galley, height) in laid.into_iter().rev() {
            let rect = at(bottom - height, height);
            if rect.top() < ceiling && !placed.is_empty() {
                break;
            }
            bottom = rect.top() - 8.0;
            placed.push((notice, galley, rect));
        }
    } else {
        let mut top = viewer.top() + 10.0;
        for (notice, galley, height) in laid {
            let rect = at(top, height);
            top = rect.bottom() + 8.0;
            placed.push((notice, galley, rect));
        }
    }
    for (notice, galley, rect) in placed {
        // The last 200 ms fade the notice out.
        let alpha = ((notice.until - now) / 0.2).clamp(0.0, 1.0) as f32;
        painter.rect_filled(
            rect,
            CornerRadius::same(metrics::MENU_RADIUS),
            p.elevated.gamma_multiply(alpha),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(metrics::MENU_RADIUS),
            Stroke::new(1.0, p.border.gamma_multiply(alpha)),
            StrokeKind::Inside,
        );
        icons::paint(
            &painter,
            Rect::from_center_size(
                pos2(rect.left() + 18.0, rect.top() + 18.0),
                Vec2::splat(14.0),
            ),
            if notice.problem {
                Icon::Warning
            } else {
                Icon::Check
            },
            if notice.problem { p.red } else { p.green }.gamma_multiply(alpha),
        );
        painter.galley_with_override_text_color(
            pos2(rect.left() + 34.0, rect.top() + 9.0),
            galley,
            p.fg.gamma_multiply(alpha),
        );
    }
}

/// Tints the window while files are held over it.
pub fn drop_target(ctx: &egui::Context, p: &Palette) {
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("drop-target"),
    ));
    painter.rect_filled(screen, CornerRadius::ZERO, theme::tint(p.accent, 0.12));
    painter.rect_stroke(
        screen.shrink(6.0),
        CornerRadius::same(metrics::PANEL_RADIUS),
        Stroke::new(2.0, p.accent),
        StrokeKind::Inside,
    );
    let pill = Rect::from_center_size(screen.center(), vec2(220.0, 44.0));
    painter.rect_filled(pill, CornerRadius::same(22), p.elevated);
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(pill.left() + 26.0, pill.center().y), Vec2::splat(16.0)),
        Icon::Import,
        p.accent,
    );
    painter.text(
        pos2(pill.left() + 44.0, pill.center().y),
        Align2::LEFT_CENTER,
        "Drop to import media",
        theme::medium(13.0),
        p.fg,
    );
}

#[cfg(test)]
#[path = "settings_sheet_tests.rs"]
mod settings_sheet_tests;

#[cfg(test)]
#[path = "notices_tests.rs"]
mod notices_tests;
