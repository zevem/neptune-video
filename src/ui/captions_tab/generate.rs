//! Captions from speech in the Captions tab: what they are made with and
//! from, the worker's progress with a way to stop it, and captions that
//! came while adding them would have cost something.
use super::{CaptionsTabState, chooser, language_menu};
use crate::{
    icons::Icon,
    languages,
    speech::{self, CueLength, Inventory, SpeechRequest, SpeechSettings, SpeechView},
    theme::{self, Palette, metrics},
    ui::{
        Action, Selection,
        widgets::{self, Kind},
    },
};
use editor_core::{Project, Time};
use editor_media::speech::{MODELS, whisper_language};
use eframe::egui::{self, CornerRadius, Id, Rect, Sense, Ui, WidgetInfo, WidgetType, pos2, vec2};

/// What the speech panel is given.
#[derive(Clone, Copy)]
pub struct SpeechInput<'a> {
    pub view: &'a SpeechView,
    pub settings: &'a SpeechSettings,
    /// What is in the models folder, once a worker has looked.
    pub inventory: Option<&'a Inventory>,
}

/// Which stretch of the timeline is heard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stretch {
    #[default]
    Timeline,
    /// From the first selected clip's start to the last one's end.
    Selection,
}

impl Stretch {
    fn name(self) -> &'static str {
        match self {
            Self::Timeline => "Whole timeline",
            Self::Selection => "Selected clips",
        }
    }

    /// The times it covers, when there is something to hear there.
    pub fn times(self, project: &Project, selection: &Selection) -> Option<(Time, Time)> {
        let (start, end) = match self {
            Self::Timeline => (Time::ZERO, project.duration()),
            Self::Selection => {
                let clips = (selection.clips.iter()).filter_map(|id| project.clip(*id));
                clips.fold(None, |span: Option<(Time, Time)>, (_, clip)| {
                    Some(match span {
                        Some((start, end)) => (start.min(clip.start), end.max(clip.end())),
                        None => (clip.start, clip.end()),
                    })
                })?
            }
        };
        (end > start).then_some((start, end))
    }
}

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

fn caption(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::medium(11.5))
            .color(p.secondary),
    );
}

/// A chooser as wide as the panel, named `name`, showing `shown`.
fn wide_chooser(ui: &mut Ui, p: &Palette, id: &str, name: &str, shown: &str) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), metrics::CONTROL_HEIGHT),
        Sense::hover(),
    );
    chooser(ui, p, rect, Id::new(id), name, shown)
}

fn model_name(settings: &SpeechSettings) -> String {
    match &settings.file {
        Some(file) => file.file_name().map_or_else(
            || file.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        None => {
            let model = settings.catalog();
            format!("{} · {} MB", model.name, model.megabytes())
        }
    }
}

/// The progress of the job under way, or the captions that wait, over the
/// top of `rect`. Gives the height it took.
pub fn status(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    view: &SpeechView,
    actions: &mut Vec<Action>,
) -> f32 {
    if matches!(view, SpeechView::Idle) {
        return 0.0;
    }
    let card = rect.shrink2(vec2(10.0, 0.0));
    let background = ui.painter().add(egui::Shape::Noop);
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(card.shrink(10.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    body.spacing_mut().item_spacing = vec2(6.0, 6.0);
    match view {
        SpeechView::Idle => {}
        SpeechView::Working {
            stage,
            done,
            stopping,
        } => {
            let text = speech::stage_text(*stage, *stopping);
            body.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cancel = ui.add_enabled_ui(!stopping, |ui| {
                        widgets::small_icon_button(ui, p, Icon::Close, "Stop making captions")
                    });
                    if cancel.inner.clicked() {
                        actions.push(Action::CancelSpeech);
                    }
                    if stage.is_some() && !stopping {
                        ui.label(
                            egui::RichText::new(format!("{} %", (done * 100.0).round() as i32))
                                .font(theme::regular(11.5))
                                .color(p.secondary),
                        );
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .font(theme::medium(12.0))
                                    .color(p.fg),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            let (bar, response) =
                body.allocate_exact_size(vec2(body.available_width(), 4.0), Sense::hover());
            response.widget_info(|| {
                let mut info =
                    WidgetInfo::labeled(WidgetType::ProgressIndicator, true, "Captions progress");
                info.value = Some(f64::from(*done));
                info
            });
            let painter = body.painter();
            painter.rect_filled(bar, CornerRadius::same(2), p.pressed);
            if stage.is_some() {
                let mut filled = bar;
                filled.set_width(bar.width() * done.clamp(0.0, 1.0));
                painter.rect_filled(filled, CornerRadius::same(2), p.accent);
            }
        }
        SpeechView::Ready {
            count,
            language,
            why,
        } => {
            body.label(
                egui::RichText::new("Captions are ready")
                    .font(theme::medium(12.5))
                    .color(p.fg),
            );
            let captions = match count {
                1 => "1 caption".to_owned(),
                count => format!("{count} captions"),
            };
            note(
                &mut body,
                p,
                &format!("{captions} in {}. {why}", languages::label(language)),
            );
            body.horizontal(|ui| {
                if widgets::button(ui, p, None, "Add captions", Kind::Primary).clicked() {
                    actions.push(Action::AddHeardCaptions);
                }
                if widgets::button(ui, p, None, "Discard", Kind::Plain)
                    .on_hover_text("Give these captions up")
                    .clicked()
                {
                    actions.push(Action::DiscardHeardCaptions);
                }
            });
        }
    }
    let used = body.min_rect().expand(10.0);
    let shape = Rect::from_min_max(card.min, pos2(card.right(), used.bottom()));
    ui.painter().set(
        background,
        egui::epaint::RectShape::filled(
            shape,
            CornerRadius::same(metrics::CONTROL_RADIUS),
            p.control,
        ),
    );
    shape.height() + 8.0
}

/// The choices captions from speech are made with, and the button that
/// makes them, in `rect`.
#[expect(clippy::too_many_arguments, reason = "the tab's one call")]
pub fn panel(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    project: &Project,
    selection: &Selection,
    state: &mut CaptionsTabState,
    input: SpeechInput,
    actions: &mut Vec<Action>,
) {
    let mut settings = input.settings.clone();
    let mut frame = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    egui::ScrollArea::vertical()
        .id_salt("caption-speech")
        .auto_shrink([false, false])
        .show(&mut frame, |outer| {
            outer.add_space(4.0);
            let mut ui = outer.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(
                        outer.cursor().min + vec2(10.0, 0.0),
                        vec2(outer.available_width() - 20.0, f32::INFINITY),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let ui = &mut ui;
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Captions from speech")
                        .font(theme::medium(13.0))
                        .color(p.fg),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::small_icon_button(ui, p, Icon::Close, "Close").clicked() {
                        state.speech_open = false;
                    }
                });
            });
            note(
                ui,
                p,
                "Made on this computer from the timeline's sound. Nothing is uploaded.",
            );
            ui.add_space(4.0);

            // The model.
            caption(ui, p, "Model");
            let model = wide_chooser(
                ui,
                p,
                "speech-model",
                "Speech model",
                &model_name(&settings),
            );
            egui::Popup::menu(&model)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    widgets::menu_style(ui, 240.0);
                    for model in &MODELS {
                        let chosen = settings.file.is_none() && settings.model == model.id;
                        let size = if input.inventory.is_some_and(|seen| seen.has(model)) {
                            "Downloaded".to_owned()
                        } else {
                            format!("{} MB", model.megabytes())
                        };
                        let item = widgets::menu_item(
                            ui,
                            p,
                            chosen.then_some(Icon::Check),
                            model.name,
                            &size,
                        )
                        .on_hover_text(model.note);
                        if item.clicked() {
                            settings.model = model.id.to_owned();
                            settings.file = None;
                            ui.close();
                        }
                    }
                    ui.separator();
                    if let Some(file) = &settings.file {
                        let name = model_name(&settings);
                        widgets::menu_item(ui, p, Some(Icon::Check), &name, "Yours")
                            .on_hover_text(file.display().to_string());
                    }
                    if widgets::menu_item(ui, p, Some(Icon::File), "Model file…", "").clicked() {
                        actions.push(Action::PickSpeechModel);
                        ui.close();
                    }
                });
            let catalog = settings.catalog();
            let missing =
                settings.file.is_none() && !input.inventory.is_some_and(|seen| seen.has(catalog));
            match &settings.file {
                Some(_) => note(ui, p, "A Whisper model file of your own."),
                None if missing => note(
                    ui,
                    p,
                    &format!(
                        "{}. Downloads {} MB once from Hugging Face, checked before use.",
                        catalog.note,
                        catalog.megabytes()
                    ),
                ),
                None => note(ui, p, &format!("{}. On this computer.", catalog.note)),
            }
            ui.add_space(2.0);

            // The language spoken.
            caption(ui, p, "Language spoken");
            let shown = (settings.language.as_deref())
                .map_or_else(|| "Detect from speech".to_owned(), languages::label);
            let language = wide_chooser(ui, p, "speech-language", "Spoken language", &shown);
            egui::Popup::menu(&language)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    widgets::menu_style(ui, 220.0);
                    let detect = settings.language.is_none().then_some(Icon::Check);
                    if widgets::menu_item(ui, p, detect, "Detect from speech", "").clicked() {
                        settings.language = None;
                        ui.close();
                    }
                    ui.separator();
                    let current = settings.language.clone();
                    if let Some(tag) = language_menu(ui, p, &mut state.language, current.as_deref())
                    {
                        settings.language = Some(tag);
                    }
                });
            if (settings.language.as_deref()).is_some_and(|tag| whisper_language(tag).is_none()) {
                note(
                    ui,
                    p,
                    "Whisper does not know this language, so it tells the language from the sound.",
                );
            }
            ui.add_space(2.0);

            // What is heard.
            caption(ui, p, "Captions from");
            let stretch = wide_chooser(
                ui,
                p,
                "speech-stretch",
                "Captions from",
                state.stretch.name(),
            );
            egui::Popup::menu(&stretch)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    widgets::menu_style(ui, 200.0);
                    for choice in [Stretch::Timeline, Stretch::Selection] {
                        let icon = (state.stretch == choice).then_some(Icon::Check);
                        if widgets::menu_item(ui, p, icon, choice.name(), "").clicked() {
                            state.stretch = choice;
                            ui.close();
                        }
                    }
                });
            let times = state.stretch.times(project, selection);
            if times.is_none() {
                note(
                    ui,
                    p,
                    match state.stretch {
                        Stretch::Timeline => "The timeline is empty.",
                        Stretch::Selection => "Select the clips to caption.",
                    },
                );
            }
            ui.add_space(2.0);

            // How long each caption is.
            caption(ui, p, "Caption length");
            let length = wide_chooser(
                ui,
                p,
                "speech-length",
                "Caption length",
                settings.length.name(),
            );
            egui::Popup::menu(&length)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    widgets::menu_style(ui, 220.0);
                    for choice in CueLength::ALL {
                        let icon = (settings.length == choice).then_some(Icon::Check);
                        if widgets::menu_item(ui, p, icon, choice.name(), choice.note()).clicked() {
                            settings.length = choice;
                            ui.close();
                        }
                    }
                });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if widgets::switch(ui, p, settings.translate, "Translate to English").clicked() {
                    settings.translate = !settings.translate;
                }
                ui.label(
                    egui::RichText::new("Translate to English")
                        .font(theme::regular(12.0))
                        .color(p.fg),
                );
            });
            ui.add_space(6.0);

            let working = input.view.working();
            let label = if missing {
                "Download and generate"
            } else {
                "Generate captions"
            };
            let pressed = ui
                .add_enabled_ui(!working && times.is_some(), |ui| {
                    widgets::button(ui, p, Some(Icon::Speech), label, Kind::Primary)
                })
                .inner;
            if working {
                note(ui, p, "Captions are being made.");
            }
            if pressed.clicked()
                && let Some((start, end)) = times
            {
                actions.push(Action::GenerateCaptions(SpeechRequest {
                    start,
                    end,
                    model: settings.choice(),
                    language: settings.language.clone(),
                    translate: settings.translate,
                    length: settings.length,
                }));
                state.speech_open = false;
            }
            ui.add_space(8.0);
            // The scroll area measures what it holds by its own cursor.
            outer.advance_cursor_after_rect(ui.min_rect());
        });
    if settings != *input.settings {
        actions.push(Action::SetSpeech(settings));
    }
}
