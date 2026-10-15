//! The agent panel: a request to the model and what it did, the command
//! line, and the live session external agents connect to.
//!
//! Nothing here pretends. "Ask" sends the request to the signed-in provider
//! through `crate::ai` and shows only what its runtime reported. As it works,
//! the agent's words and each tool step it takes appear as they happen, and
//! its edits land in the project at once. For review, its proposal edits
//! nothing until it is applied. "Command" runs the exact language in
//! `crate::agent` and asks no model. All end in the same commands as every
//! other edit.
use super::{
    Action,
    widgets::{self, Kind, icon_button, menu_item, menu_style},
};
use crate::{
    agent::EXAMPLES,
    ai::{AiSettings, Effort, Plan, Provider, STEPS_SHOWN},
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{self, Align2, CornerRadius, Key, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::path::Path;

const LOG_LIMIT: usize = 200;
const SESSION_HEIGHT: f32 = 54.0;
const COMPOSER_HEIGHT: f32 = 86.0;

pub enum Entry {
    /// A line typed for the command language.
    Asked(String),
    /// What the model was asked.
    Prompt(String),
    /// What came of it, one line per action.
    Answer {
        ok: bool,
        lines: Vec<String>,
    },
    /// A request that arrived over the live session.
    External {
        ok: bool,
        text: String,
    },
    /// What the model proposed, and what became of it.
    Plan(PlanCard),
    /// What the agent said while it worked.
    Said(String),
    /// One tool step the agent took.
    Step(Step),
    /// Something that happened and needs no mark.
    Note(String),
    Help,
}

pub struct Step {
    pub call: String,
    pub what: String,
    pub state: StepState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StepState {
    Running,
    /// Done; the edit it made, when it made one.
    Done(Option<String>),
    /// Refused, and why when the tool said.
    Refused(Option<String>),
    /// The request ended before the step did.
    Stopped,
}

pub struct PlanCard {
    pub plan: Plan,
    pub state: PlanState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanState {
    /// Waiting for the person to apply or discard it.
    Open,
    /// The model answered without proposing a change.
    Empty,
    Applied,
    /// Applied, and the project already was what the edits make it: nothing
    /// was added to the history.
    Unchanged,
    Discarded,
    /// It can no longer be applied, and why.
    Closed(String),
}

impl PlanState {
    /// The line under a proposal that has been decided.
    pub fn said(&self) -> &str {
        match self {
            Self::Open => "",
            Self::Empty => "No edit was proposed.",
            Self::Applied => "Applied. Undo takes it back in one step.",
            Self::Unchanged => "No changes. The project already matched these edits.",
            Self::Discarded => "Discarded. Nothing was edited.",
            Self::Closed(reason) => reason,
        }
    }
}

#[derive(Default)]
pub struct AgentState {
    pub input: String,
    pub log: Vec<Entry>,
    /// Give the field the keyboard on the next frame.
    pub focus: bool,
    /// Scroll to the newest entry on the next frame.
    stick: bool,
}

impl AgentState {
    pub fn push(&mut self, entry: Entry) {
        self.log.push(entry);
        if self.log.len() > LOG_LIMIT {
            let extra = self.log.len() - LOG_LIMIT;
            self.log.drain(..extra);
        }
        self.stick = true;
    }

    /// Records how the tool step `call` ended.
    pub fn step_ended(&mut self, call: &str, state: StepState) {
        let step = self.log.iter_mut().rev().find_map(|entry| match entry {
            Entry::Step(step) if step.call == call && step.state == StepState::Running => {
                Some(step)
            }
            _ => None,
        });
        if let Some(step) = step {
            step.state = state;
        }
    }

    /// The request ended: steps still running did not finish.
    pub fn steps_stopped(&mut self) {
        for entry in &mut self.log {
            if let Entry::Step(step) = entry
                && step.state == StepState::Running
            {
                step.state = StepState::Stopped;
            }
        }
    }

    /// Records what became of the proposal request `id` brought back.
    pub fn settle(&mut self, id: uuid::Uuid, state: PlanState) {
        let card = self.log.iter_mut().rev().find_map(|entry| match entry {
            Entry::Plan(card) if card.plan.id == id => Some(card),
            _ => None,
        });
        if let Some(card) = card {
            card.state = state;
        }
    }
}

/// A request the model has not answered yet.
pub struct WaitingInput<'a> {
    pub provider: &'a str,
    pub seconds: u64,
    /// The provider's process is running.
    pub started: bool,
    pub cancelling: bool,
    /// The agent is editing as it works, not planning for review.
    pub working: bool,
    /// Edits it made so far.
    pub edits: usize,
}

pub struct AgentInput<'a> {
    pub p: &'a Palette,
    /// Where this window's session descriptor was published.
    pub session: Option<&'a Path>,
    /// Why there is no session, when there is none.
    pub session_problem: Option<&'a str>,
    /// The field asks the model; otherwise it runs a command.
    pub ask: bool,
    pub ai: &'a AiSettings,
    pub waiting: Option<WaitingInput<'a>>,
    /// The open proposal is still about the project as it stands.
    pub current: bool,
    /// A conversation goes on: the next request continues it.
    pub talking: bool,
}

pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut AgentState,
    input: &AgentInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let session = Rect::from_min_size(rect.min, vec2(rect.width(), SESSION_HEIGHT));
    let composer = Rect::from_min_max(pos2(rect.left(), rect.bottom() - COMPOSER_HEIGHT), rect.max);
    let log = Rect::from_min_max(
        pos2(rect.left(), session.bottom()),
        pos2(rect.right(), composer.top()),
    );

    session_row(ui, session, input, actions);

    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(log.shrink2(vec2(12.0, 0.0))));
    body.set_clip_rect(log);
    let stick = std::mem::take(&mut state.stick);
    egui::ScrollArea::vertical()
        .id_salt("agent-log")
        .auto_shrink([false, false])
        // What is said last is what is read; an empty log is the
        // introduction, which is read from its top.
        .stick_to_bottom(!state.log.is_empty())
        .show(&mut body, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add_space(8.0);
            if state.log.is_empty() {
                introduction(ui, p, input);
            }
            for entry in &state.log {
                match entry {
                    Entry::Asked(text) => asked(ui, p, text, theme::mono(12.0)),
                    Entry::Prompt(text) => asked(ui, p, text, theme::regular(12.5)),
                    Entry::Answer { ok, lines } => answer(ui, p, *ok, lines),
                    Entry::External { ok, text } => external(ui, p, *ok, text),
                    Entry::Plan(card) => plan(ui, p, card, input.current, actions),
                    Entry::Said(text) => said(ui, p, text),
                    Entry::Step(step) => step_row(ui, p, step),
                    Entry::Note(text) => note(ui, p, text),
                    Entry::Help => examples(ui, p),
                }
            }
            if let Some(waiting) = &input.waiting {
                waiting_row(ui, p, waiting, actions);
            }
            ui.add_space(4.0);
            if stick {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });

    ui.painter()
        .hline(rect.x_range(), composer.top(), p.hairline());
    let bar = Rect::from_min_size(
        pos2(composer.left() + 12.0, composer.top() + 8.0),
        vec2(composer.width() - 24.0, metrics::CONTROL_HEIGHT),
    );
    mode_bar(ui, bar, input, actions);

    // The field: a request to the model, or a line of the command language.
    let field = Rect::from_min_max(
        pos2(bar.left(), bar.bottom() + 6.0),
        pos2(bar.right(), composer.bottom() - 12.0),
    );
    ui.painter().rect_filled(
        field,
        CornerRadius::same(metrics::CONTROL_RADIUS + 2),
        p.control,
    );
    let send = Rect::from_center_size(
        pos2(field.right() - 16.0, field.center().y),
        Vec2::splat(24.0),
    );
    let mut edit = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
        pos2(field.left() + 10.0, field.top() + 6.0),
        pos2(send.left() - 6.0, field.bottom() - 6.0),
    )));
    let response = edit.add(
        egui::TextEdit::singleline(&mut state.input)
            .hint_text(if input.ask && input.talking {
                "Reply, or ask for more"
            } else if input.ask {
                "Describe an edit"
            } else {
                "Type a command, or “help”"
            })
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .font(theme::regular(13.0)),
    );
    if std::mem::take(&mut state.focus) {
        response.request_focus();
    }
    // One request at a time: while the model is asked, the button stops it.
    let stop = input.ask && input.waiting.is_some();
    let stopping = input.waiting.as_ref().is_some_and(|w| w.cancelling);
    let ready = if stop {
        !stopping
    } else {
        !state.input.trim().is_empty()
    };
    let label = if stop {
        "Stop the request"
    } else if input.ask {
        "Ask (Enter)"
    } else {
        "Run command (Enter)"
    };
    let entered = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let button = ui.interact(send, ui.id().with("agent-send"), Sense::click());
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready, label));
    ui.painter().circle_filled(
        send.center(),
        12.0,
        if ready { p.accent } else { p.pressed },
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(send.center(), Vec2::splat(13.0)),
        if stop { Icon::Stop } else { Icon::ArrowUp },
        if ready { p.on_accent } else { p.muted },
    );
    let clicked = button.on_hover_text(label).clicked();
    if stop {
        if ready && clicked {
            actions.push(Action::AiCancel);
        }
        if entered {
            // The words stay for when the request is over.
            state.focus = true;
        }
    } else if ready && (entered || clicked) {
        let text = std::mem::take(&mut state.input);
        actions.push(if input.ask {
            Action::AiSubmit(text)
        } else {
            Action::AgentSubmit(text)
        });
        // Enter keeps the keyboard in the field for the next line.
        state.focus = true;
    }
}

/// What the field does, and who answers when it asks.
fn mode_bar(ui: &mut Ui, rect: Rect, input: &AgentInput, actions: &mut Vec<Action>) {
    let p = input.p;
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut bar;
    let mut ask = input.ask;
    if widgets::segmented(ui, p, &mut ask, &[(true, "Ask"), (false, "Command")], 136.0) {
        actions.push(Action::SetAgentAsk(ask));
    }
    if !input.ask {
        return;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let described = input.ai.describe();
        let room = (ui.available_width() - 6.0 - 26.0).max(24.0);
        let galley = widgets::truncated(ui, &described, theme::medium(11.5), p.secondary, room);
        let size = vec2(galley.size().x + 26.0, 24.0);
        let (chip, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Model: {described}"),
            )
        });
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
        if response.hovered() || open {
            ui.painter()
                .rect_filled(chip, CornerRadius::same(metrics::CONTROL_RADIUS), p.hover);
        }
        ui.painter().galley(
            pos2(chip.left() + 8.0, chip.center().y - galley.size().y * 0.5),
            galley,
            p.secondary,
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(chip.right() - 10.0, chip.center().y), Vec2::splat(10.0)),
            Icon::ChevronDown,
            p.muted,
        );
        egui::Popup::menu(&response).show(|ui| {
            menu_style(ui, 210.0);
            let heading = |ui: &mut Ui, text: &str| {
                ui.label(
                    egui::RichText::new(text)
                        .font(theme::medium(11.0))
                        .color(p.muted),
                );
            };
            let chosen = |on: bool| on.then_some(Icon::Check);
            heading(ui, "Answered by");
            for provider in Provider::ALL {
                let on = input.ai.provider == provider;
                if menu_item(ui, p, chosen(on), provider.name(), "").clicked() {
                    actions.push(Action::SetAiProvider(provider));
                    ui.close();
                }
            }
            ui.separator();
            heading(ui, "Effort");
            for effort in Effort::ALL {
                let on = input.ai.effort == effort;
                if menu_item(ui, p, chosen(on), effort.name(), "").clicked() {
                    actions.push(Action::SetAiEffort(effort));
                    ui.close();
                }
            }
            ui.separator();
            heading(ui, "Edits");
            for (review, name) in [(false, "Made as it works"), (true, "Proposed for review")] {
                let on = input.ai.review == review;
                if menu_item(ui, p, chosen(on), name, "").clicked() {
                    actions.push(Action::SetAiReview(review));
                    ui.close();
                }
            }
            ui.separator();
            if input.talking
                && menu_item(ui, p, Some(Icon::Plus), "New conversation", "").clicked()
            {
                actions.push(Action::AiNewConversation);
                ui.close();
            }
            if menu_item(ui, p, Some(Icon::Settings), "Model…", "").clicked() {
                actions.push(Action::OpenSettings);
                ui.close();
            }
        });
        response.on_hover_text(
            "Who answers. It uses the sign-in of the Claude or Codex command line on this computer.",
        );
    });
}

fn session_row(ui: &mut Ui, rect: Rect, input: &AgentInput, actions: &mut Vec<Action>) {
    let p = input.p;
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), p.hairline());
    let live = input.session.is_some();
    let dot = pos2(rect.left() + 20.0, rect.top() + 19.0);
    if live {
        ui.painter()
            .circle_filled(dot, 7.0, theme::tint(p.green, 0.2));
    }
    ui.painter()
        .circle_filled(dot, 3.5, if live { p.green } else { p.muted });
    ui.painter().text(
        pos2(rect.left() + 34.0, rect.top() + 19.0),
        Align2::LEFT_CENTER,
        if live {
            "Agent session is open"
        } else {
            "No agent session"
        },
        theme::medium(12.5),
        p.fg,
    );
    let detail = match (input.session, input.session_problem) {
        (Some(path), _) => format!("aster-agent --connect {}", path.display()),
        (None, Some(problem)) => problem.to_owned(),
        (None, None) => "The session could not be opened.".to_owned(),
    };
    let galley = widgets::truncated(
        ui,
        &detail,
        theme::mono(10.5),
        p.muted,
        rect.width() - 34.0 - 44.0,
    );
    ui.painter()
        .galley(pos2(rect.left() + 34.0, rect.top() + 31.0), galley, p.muted);

    let mut trailing = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if live {
        let copy = icon_button(&mut trailing, p, Icon::Copy, "Copy the connect command");
        if copy.clicked() {
            trailing.ctx().copy_text(detail);
            actions.push(Action::Notify("Connect command copied".into()));
        }
    }
}

fn introduction(ui: &mut Ui, p: &Palette, input: &AgentInput) {
    ui.add_space(6.0);
    let title = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .font(theme::medium(13.0))
                .color(p.fg),
        );
    };
    let body = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .font(theme::regular(12.0))
                .color(p.secondary),
        );
    };
    if input.ask && !input.ai.review {
        title(ui, "Ask the agent");
        body(
            ui,
            &format!(
                "{} edits this project with Aster's own tools while you watch: it can \
                 look at frames, import media, cut and arrange clips, add effects, \
                 titles and captions, transcribe speech, mix sound and export. Each \
                 edit appears here as it is made and is one step you can undo.",
                input.ai.provider.name()
            ),
        );
        body(
            ui,
            "It sees the frames, transcripts and sound levels it asks for, and \
             the files in this project's folders, in your Movies, Music, \
             Pictures, Desktop, Documents and Downloads folders and in folders \
             you add in Settings. It can also make, open and export other \
             projects. Follow-up requests continue the conversation.",
        );
    } else if input.ask {
        title(ui, "Ask for an edit");
        body(
            ui,
            &format!(
                "Describe a change and {} plans it as ordinary editing commands. \
                 Nothing is edited until you apply the plan, and applying it is one \
                 step you can undo.",
                input.ai.provider.name()
            ),
        );
        body(
            ui,
            "The request carries this project's editing data: names, times and \
             settings. Your media files are not sent.",
        );
    } else {
        title(ui, "Edit by command");
        body(
            ui,
            "These lines are run here as they are; no model is asked. An agent \
             that connects to the session above edits with the same commands, \
             and every change can be undone.",
        );
        examples(ui, p);
    }
}

fn examples(ui: &mut Ui, p: &Palette) {
    egui::Frame::NONE
        .fill(p.control)
        .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS + 2))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            for (command, meaning) in EXAMPLES {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        egui::RichText::new(command)
                            .font(theme::mono(11.5))
                            .color(p.fg),
                    );
                    ui.label(
                        egui::RichText::new(meaning)
                            .font(theme::regular(11.5))
                            .color(p.muted),
                    );
                });
            }
            ui.label(
                egui::RichText::new("A line that starts with { is applied as a JSON command.")
                    .font(theme::regular(11.5))
                    .color(p.muted),
            );
        });
}

/// The person's line: a filled bubble at the trailing edge.
fn asked(ui: &mut Ui, p: &Palette, text: &str, font: egui::FontId) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
        egui::Frame::NONE
            .fill(theme::tint(p.accent, if p.dark { 0.28 } else { 0.16 }))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_max_width((ui.available_width() - 24.0).max(60.0));
                ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(p.fg)).wrap());
            });
    });
}

fn mark(ui: &mut Ui, ok: bool, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    icons::paint(
        ui.painter(),
        rect.shrink(1.0),
        if ok { Icon::Check } else { Icon::Warning },
        if ok { p.green } else { p.red },
    );
}

/// What came of a line: one row per action, each with its mark and words.
fn answer(ui: &mut Ui, p: &Palette, ok: bool, lines: &[String]) {
    for line in lines {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            mark(ui, ok, p);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(line)
                        .font(theme::regular(12.5))
                        .color(if ok { p.fg } else { p.secondary }),
                )
                .wrap(),
            );
        });
    }
}

/// A request from a connected agent: a muted one-line row.
fn external(ui: &mut Ui, p: &Palette, ok: bool, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(
            ui.painter(),
            rect.shrink(1.0),
            if ok { Icon::Sparkles } else { Icon::Warning },
            if ok { p.accent } else { p.red },
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(theme::regular(12.0))
                    .color(if ok { p.secondary } else { p.red }),
            )
            .wrap(),
        );
    });
}

fn note(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::regular(12.0))
            .color(p.muted),
    );
}

/// What the agent said while it worked.
fn said(ui: &mut Ui, p: &Palette, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(ui.painter(), rect.shrink(1.0), Icon::Sparkles, p.accent);
        ui.add(egui::Label::new(written(text, p)).wrap());
    });
}

/// The agent's words with the little Markdown models write: **strong**,
/// `code`, headings and bullets. Anything else is shown as it was written.
fn written(text: &str, p: &Palette) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let plain = egui::TextFormat::simple(theme::regular(12.5), p.fg);
    let strong = egui::TextFormat::simple(theme::medium(12.5), p.fg);
    let code = egui::TextFormat::simple(theme::mono(11.5), p.secondary);
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            job.append("\n", 0.0, plain.clone());
        }
        let trimmed = line.trim_start();
        let (line, heading) = match trimmed.strip_prefix('#') {
            Some(_) => (trimmed.trim_start_matches('#').trim_start(), true),
            None => (line, false),
        };
        let line = match trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            Some(rest) if !heading => {
                job.append("•  ", 0.0, plain.clone());
                rest
            }
            _ => line,
        };
        let mut bold = heading;
        let mut mono = false;
        let mut rest = line;
        while !rest.is_empty() {
            let next = [rest.find("**"), rest.find('`')]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(rest.len());
            let format = if mono {
                &code
            } else if bold {
                &strong
            } else {
                &plain
            };
            if next > 0 {
                job.append(&rest[..next], 0.0, format.clone());
            }
            rest = &rest[next..];
            if let Some(after) = rest.strip_prefix("**").filter(|_| !mono) {
                bold = !bold || heading;
                rest = after;
            } else if let Some(after) = rest.strip_prefix('`') {
                mono = !mono;
                rest = after;
            } else if !rest.is_empty() {
                // `**` inside code: shown as it is.
                job.append(&rest[..2], 0.0, code.clone());
                rest = &rest[2..];
            }
        }
    }
    job
}

/// One tool step: what it was and how it ended, with the edit it made.
fn step_row(ui: &mut Ui, p: &Palette, step: &Step) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        let (detail, ink) = match &step.state {
            StepState::Running => {
                ui.put(rect, egui::Spinner::new().size(11.0).color(p.muted));
                (None, p.secondary)
            }
            StepState::Done(edit) => {
                icons::paint(ui.painter(), rect.shrink(2.0), Icon::Check, p.green);
                (edit.as_deref(), p.secondary)
            }
            StepState::Refused(why) => {
                icons::paint(ui.painter(), rect.shrink(1.0), Icon::Warning, p.yellow);
                (why.as_deref(), p.secondary)
            }
            StepState::Stopped => {
                icons::paint(ui.painter(), rect.shrink(2.0), Icon::Close, p.muted);
                (None, p.muted)
            }
        };
        let mut text = egui::text::LayoutJob::default();
        text.append(
            &step.what,
            0.0,
            egui::TextFormat::simple(theme::regular(12.0), ink),
        );
        if let Some(detail) = detail {
            text.append(
                &format!("  {detail}"),
                0.0,
                egui::TextFormat::simple(theme::regular(12.0), p.muted),
            );
        }
        ui.add(egui::Label::new(text).wrap());
    });
}

/// The request in flight: that it is, for how long, and the way to stop it.
fn waiting_row(ui: &mut Ui, p: &Palette, waiting: &WaitingInput, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::CONTROL_HEIGHT);
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(egui::Spinner::new().size(12.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cancel = ui
                .add_enabled_ui(!waiting.cancelling, |ui| {
                    widgets::button(ui, p, None, "Cancel", Kind::Plain)
                })
                .inner;
            if cancel.clicked() {
                actions.push(Action::AiCancel);
            }
            let text = if waiting.cancelling {
                "Cancelling…".to_owned()
            } else if !waiting.started {
                format!("Starting {}…", waiting.provider)
            } else if waiting.working {
                // The picker names the provider; the time and the count
                // are what must fit beside Cancel.
                match waiting.edits {
                    0 => format!("Working… {}s", waiting.seconds),
                    1 => format!("Working… {}s · 1 change", waiting.seconds),
                    n => format!("Working… {}s · {n} changes", waiting.seconds),
                }
            } else {
                format!("{} is planning… {}s", waiting.provider, waiting.seconds)
            };
            let room = ui.available_width() - 4.0;
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let galley = widgets::truncated(ui, &text, theme::regular(12.5), p.secondary, room);
                let (rect, _) = ui.allocate_exact_size(
                    vec2(room.max(0.0), metrics::CONTROL_HEIGHT),
                    Sense::hover(),
                );
                ui.painter().galley(
                    pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
                    galley,
                    p.secondary,
                );
            });
        });
    });
    // The seconds are read off a clock; the spinner alone would not say so.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
}

/// A proposal: what the model says it would do, the commands that do it,
/// what they change, and the decision.
fn plan(ui: &mut Ui, p: &Palette, card: &PlanCard, current: bool, actions: &mut Vec<Action>) {
    let plan = &card.plan;
    let small = |ui: &mut Ui, icon: Icon, tint: egui::Color32, text: &str, ink: egui::Color32| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
            icons::paint(ui.painter(), rect.shrink(1.0), icon, tint);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .font(theme::regular(12.0))
                        .color(ink),
                )
                .wrap(),
            );
        });
    };
    ui.push_id(plan.id, |ui| {
        egui::Frame::NONE
            .fill(p.control)
            .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS + 2))
            .inner_margin(egui::Margin::symmetric(10, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                small(ui, Icon::Sparkles, p.accent, &plan.by, p.muted);
                if !plan.summary.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&plan.summary)
                                .font(theme::regular(12.5))
                                .color(p.fg),
                        )
                        .wrap(),
                    );
                }
                if !plan.steps.is_empty() {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (index, step) in plan.steps.iter().take(STEPS_SHOWN).enumerate() {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
                                ui.painter().text(
                                    rect.right_center(),
                                    Align2::RIGHT_CENTER,
                                    (index + 1).to_string(),
                                    theme::mono(10.5),
                                    p.muted,
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(step)
                                            .font(theme::regular(12.0))
                                            .color(p.secondary),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                        if plan.steps.len() > STEPS_SHOWN {
                            ui.label(
                                egui::RichText::new(format!(
                                    "and {} more",
                                    plan.steps.len() - STEPS_SHOWN
                                ))
                                .font(theme::regular(11.5))
                                .color(p.muted),
                            );
                        }
                    });
                }
                if !plan.changes.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(plan.changes.join("  ·  "))
                                .font(theme::regular(11.5))
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                }
                match &card.state {
                    PlanState::Open => {
                        if !current {
                            small(
                                ui,
                                Icon::Warning,
                                p.yellow,
                                "The project changed since this was planned. Ask again.",
                                p.secondary,
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let apply = ui
                                        .add_enabled_ui(current, |ui| {
                                            widgets::button(
                                                ui,
                                                p,
                                                Some(Icon::Check),
                                                "Apply",
                                                Kind::Primary,
                                            )
                                        })
                                        .inner
                                        .on_hover_text("Make these edits as one step you can undo")
                                        .on_disabled_hover_text(
                                            "This plan is about an earlier state of the project",
                                        );
                                    if apply.clicked() {
                                        actions.push(Action::AiApply(plan.id));
                                    }
                                    let discard =
                                        widgets::button(ui, p, None, "Discard", Kind::Plain)
                                            .on_hover_text("Close this plan; nothing is edited");
                                    if discard.clicked() {
                                        actions.push(Action::AiDiscard(plan.id));
                                    }
                                },
                            );
                        });
                    }
                    state @ (PlanState::Empty | PlanState::Unchanged) => {
                        small(ui, Icon::Check, p.muted, state.said(), p.muted)
                    }
                    state @ PlanState::Applied => {
                        small(ui, Icon::Check, p.green, state.said(), p.secondary)
                    }
                    state @ PlanState::Discarded => {
                        small(ui, Icon::Close, p.muted, state.said(), p.muted)
                    }
                    state @ PlanState::Closed(_) => {
                        small(ui, Icon::Warning, p.red, state.said(), p.secondary)
                    }
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: uuid::Uuid) -> Entry {
        Entry::Plan(PlanCard {
            plan: Plan {
                id,
                summary: "Tighten the opening".into(),
                steps: vec!["Trim clip".into()],
                changes: Vec::new(),
                by: "Claude".into(),
            },
            state: PlanState::Open,
        })
    }

    #[test]
    fn markdown_marks_are_read_not_shown() {
        let p = Palette::new(theme::Appearance::Dark, theme::Accent::default());
        let job = written("## Done\n- **Exports:** `promo.mp4` is ready", &p);
        assert_eq!(job.text, "Done\n•  Exports: promo.mp4 is ready");
        let unclosed = written("a `b ** c` d", &p);
        assert_eq!(unclosed.text, "a b ** c d");
    }

    #[test]
    fn a_step_ends_once_and_steps_left_running_stop_with_the_request() {
        let mut state = AgentState::default();
        for call in ["a", "b"] {
            state.push(Entry::Step(Step {
                call: call.into(),
                what: "Edited clips".into(),
                state: StepState::Running,
            }));
        }
        state.step_ended("a", StepState::Done(Some("Trim clip".into())));
        state.step_ended("a", StepState::Refused(None));
        state.steps_stopped();
        let states: Vec<_> = (state.log.iter())
            .filter_map(|entry| match entry {
                Entry::Step(step) => Some(step.state.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            states,
            [
                StepState::Done(Some("Trim clip".into())),
                StepState::Stopped
            ]
        );
    }

    #[test]
    fn a_decision_settles_the_proposal_it_was_made_about() {
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let mut state = AgentState::default();
        state.push(card(first));
        state.push(Entry::Note("Request cancelled".into()));
        state.push(card(second));
        state.settle(first, PlanState::Applied);
        // A proposal that is not in the log settles nothing.
        state.settle(uuid::Uuid::new_v4(), PlanState::Discarded);
        let states: Vec<_> = state
            .log
            .iter()
            .filter_map(|entry| match entry {
                Entry::Plan(card) => Some(card.state.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(states, [PlanState::Applied, PlanState::Open]);
    }
}
