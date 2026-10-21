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
//!
//! It is laid out as Neptune Terminal's chat: a row that says whether the
//! session is open, the conversation, and one composer surface at the foot
//! that holds the field, what it does and who answers, and the round button
//! that sends.
use super::{
    Action, PreferencesPane,
    controls::{self, ButtonKind, ButtonSize, elided, focus_ring, galley_at, place, viewport_id},
    kit,
    widgets::{self, menu_item, menu_style},
};
use crate::{
    agent::EXAMPLES,
    ai::{AiSettings, Effort, Plan, Provider, STEPS_SHOWN},
    icons::{self, Icon},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Align, Color32, CursorIcon, FontId, Id, Key, KeyboardShortcut, Layout, Modifiers, Pos2,
    Rect, RectAlign, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo,
    WidgetType, pos2, text::LayoutJob, vec2,
};
use std::path::Path;

const LOG_LIMIT: usize = 200;
/// The row that says whether a session is open, over its hairline.
const SESSION: f32 = 30.0;
/// Where the session's dot, and every mark of the conversation, is centred
/// from the panel's leading edge, and where the words after them begin.
const MARK_CENTRE: f32 = 10.0;
const MARKED: f32 = 24.0;
/// Words without a mark stand this far in from either edge.
const TEXT_INSET: f32 = 6.0;
/// The share of the width a line of the person's takes at most.
const BUBBLE: f32 = 0.86;

/// The composer's corner, as Terminal's: its controls' corner and their
/// inset from its edge.
const RADIUS: u8 = 14;
const INSET: f32 = 8.0;
/// The row of controls at the surface's foot, and the room over the words
/// and beside them: they begin where the row's switch does.
const FOOT: f32 = 30.0;
const TOP: f32 = 12.0;
const SIDE: f32 = INSET;
/// The room between the words and the row under them.
const GAP: f32 = 4.0;
/// The most lines the field shows before it scrolls.
const LINES: f32 = 6.0;
/// The round button that sends, and the switch between asking and commands:
/// its own width, narrowed toward its labels' fit only to keep who answers
/// whole, and as wide as it takes where the row has room.
const ROUND: f32 = 28.0;
const MODE: f32 = 136.0;
const ROOMY_MODE: f32 = 152.0;
/// A pill's room around its words, and its chevron.
const PAD: f32 = 8.0;
const CHEVRON: f32 = 14.0;
const PILL_RADIUS: u8 = 6;

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

    /// The word at the head of its card.
    fn word(&self) -> &str {
        match self {
            Self::Open => "Proposed",
            Self::Empty => "No edit",
            Self::Applied => "Applied",
            Self::Unchanged => "No changes",
            Self::Discarded => "Discarded",
            Self::Closed(_) => "Closed",
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
    /// The arrows changed Ask or Command last frame: the keyboard stays on
    /// the choice rather than going back to the field the change asks for.
    keyed: bool,
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
    /// A proposal waits for the person to apply or discard it: the planner
    /// says so, as it does for the Agent tab's count, whether or not its
    /// card is still in the log.
    pub proposal_waits: bool,
}

pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut AgentState,
    input: &AgentInput,
    actions: &mut Vec<Action>,
) {
    let session = Rect::from_min_size(rect.min, vec2(rect.width(), SESSION));
    session_row(ui, session, input, actions);

    // The composer grows with what is written in it, up to six lines, and
    // the conversation takes what it leaves.
    let font = field_font(input.ask);
    let height = composer_height(ui, &state.input, rect.width() - 2.0 * SIDE, &font);
    let top = (rect.bottom() - INSET - height).max(session.bottom() + INSET);
    let composer = Rect::from_min_max(
        pos2(rect.left(), top),
        pos2(rect.right(), rect.bottom() - INSET),
    );
    let log = Rect::from_min_max(
        pos2(rect.left(), session.bottom()),
        pos2(rect.right(), (composer.top() - INSET).max(session.bottom())),
    );
    conversation(ui, log, state, input, actions);
    composer_surface(ui, composer, state, input, actions);
}

/// The field's words: the request in the interface's face, a command in the
/// face commands are written in.
fn field_font(ask: bool) -> FontId {
    if ask {
        theme::regular(13.0)
    } else {
        theme::mono(12.0)
    }
}

/// The composer's height for `text` in a field `width` wide: one line to
/// write on at least, six before it scrolls, and the row under them.
fn composer_height(ui: &Ui, text: &str, width: f32, font: &FontId) -> f32 {
    // A line of the request's face in either mode, so the composer keeps its
    // height as the mode changes.
    let line = ui.fonts_mut(|fonts| fonts.row_height(&theme::regular(13.0)));
    let words = (ui.painter())
        .layout(
            text.to_owned(),
            font.clone(),
            Color32::PLACEHOLDER,
            width.max(40.0),
        )
        .size()
        .y
        .clamp(line, line * LINES);
    TOP + words + GAP + FOOT + (INSET - 1.0)
}

// ---- The session row ------------------------------------------------------

/// Whether an agent can connect, as a dot and its words, and "More", which
/// holds the command it connects with.
fn session_row(ui: &mut Ui, rect: Rect, input: &AgentInput, actions: &mut Vec<Action>) {
    let p = input.p;
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, p.hairline());
    let (dot, words) = match (input.session, input.session_problem) {
        (Some(_), _) => (p.green, "Agent session is open"),
        (None, Some(_)) => (p.red, "The session could not be opened."),
        (None, None) => (p.muted, "No agent session"),
    };
    // A proposal waiting for an answer is what needs the person here,
    // unless the session failed: that stays red. A fill, so the plain
    // attention, as the inspector toggle's dot.
    let failed = input.session.is_none() && input.session_problem.is_some();
    let dot = if input.proposal_waits && !failed {
        p.attention
    } else {
        dot
    };
    let line = rect.center().y;
    ui.painter()
        .circle_filled(pos2(rect.left() + MARK_CENTRE, line), 4.2, dot);

    let more = Rect::from_center_size(pos2(rect.right() - ROUND * 0.5, line), Vec2::splat(ROUND));
    let galley = elided(
        ui.painter(),
        words,
        theme::regular(12.0),
        p.secondary,
        more.left() - 8.0 - (rect.left() + MARKED),
    );
    let shown = galley_at(ui.painter(), pos2(rect.left() + MARKED, line), galley);
    let said = ui.interact(shown, ui.id().with("agent-session"), Sense::hover());
    said.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, words));
    // Why it failed is read with the words, and said again where the
    // command would be copied.
    let problem = input.session_problem.filter(|_| input.session.is_none());
    if let Some(problem) = problem {
        ui.ctx()
            .accesskit_node_builder(said.id, |node| node.set_description(problem));
        said.on_hover_text(problem);
    }

    let command = input
        .session
        .map(|path| format!("neptune-video-agent --connect {}", path.display()));
    let response = ui.interact(more, ui.id().with("agent-more"), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "More"));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    kit::paint_icon_button(ui, p, &response, Icon::Dots, open, None);
    let response = icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        "More",
        "",
    );
    egui::Popup::menu(&response)
        .align(RectAlign::BOTTOM_END)
        .show(|ui| {
            menu_style(ui, 240.0);
            if let Some(problem) = problem {
                kit::menu_note(ui, p, problem);
            }
            let copy = ui
                .add_enabled_ui(command.is_some(), |ui| {
                    menu_item(ui, p, Some(Icon::Copy), "Copy the connect command", "")
                })
                .inner
                .on_disabled_hover_text(input.session_problem.unwrap_or("No agent session"));
            let copy = match &command {
                Some(command) => copy.on_hover_text(command),
                None => copy,
            };
            if copy.clicked()
                && let Some(command) = &command
            {
                ui.ctx().copy_text(command.clone());
                actions.push(Action::Notify("Connect command copied".into()));
                ui.close();
            }
            if input.talking {
                widgets::menu_separator(ui, p);
                if menu_item(ui, p, Some(Icon::MessagePlus), "New conversation", "").clicked() {
                    actions.push(Action::AiNewConversation);
                    ui.close();
                }
            }
        });
}

// ---- The conversation -----------------------------------------------------

/// What kind of thing an entry is, for the room left before it: the steps
/// of one turn stand together, everything else is set apart.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Step,
    Other,
}

fn conversation(
    ui: &mut Ui,
    log: Rect,
    state: &mut AgentState,
    input: &AgentInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    if log.height() <= 0.0 {
        return;
    }
    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(log)
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(log.intersect(ui.clip_rect()));
    let stick = std::mem::take(&mut state.stick);
    let output = egui::ScrollArea::vertical()
        .id_salt("agent-log")
        .auto_shrink([false, false])
        // What is said last is what is read; an empty log is the
        // introduction, which is read from its top.
        .stick_to_bottom(!state.log.is_empty())
        .show(&mut body, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(12.0);
            if state.log.is_empty() {
                introduction(ui, p, input);
            }
            let mut last = None;
            for entry in &state.log {
                let kind = match entry {
                    Entry::Step(_) => Kind::Step,
                    _ => Kind::Other,
                };
                if let Some(last) = last {
                    ui.add_space(if last == Kind::Step && kind == Kind::Step {
                        0.0
                    } else {
                        8.0
                    });
                }
                last = Some(kind);
                match entry {
                    Entry::Asked(text) => bubble(ui, p, text, theme::mono(12.0)),
                    Entry::Prompt(text) => bubble(ui, p, text, theme::regular(13.0)),
                    Entry::Answer { ok, lines } => answer(ui, p, *ok, lines),
                    Entry::External { ok, text } => external(ui, p, *ok, text),
                    Entry::Plan(card) => plan(ui, p, card, input.current, actions),
                    Entry::Said(text) => {
                        marked(
                            ui,
                            Mark::Glyph(Icon::Sparkles, p.secondary),
                            written(text, p),
                            0.0,
                        );
                    }
                    Entry::Step(step) => step_row(ui, p, step),
                    Entry::Note(text) => {
                        words(ui, text, theme::regular(12.0), p.secondary);
                    }
                    Entry::Help => examples(ui, p),
                }
            }
            if let Some(waiting) = &input.waiting {
                if last.is_some() {
                    ui.add_space(8.0);
                }
                waiting_row(ui, p, waiting, actions);
            }
            ui.add_space(12.0);
            if stick {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
    // Words that run on under the composer, or above under the session's
    // row, say so.
    kit::scroll_fades(ui, &output, p.chrome);
}

/// What the panel says before anything is asked: what the field does, what
/// is sent and what is seen.
fn introduction(ui: &mut Ui, p: &Palette, input: &AgentInput) {
    let title = |ui: &mut Ui, text: &str| {
        words(ui, text, theme::medium(13.0), p.fg);
        ui.add_space(6.0);
    };
    let body = |ui: &mut Ui, text: &str| {
        words(ui, text, theme::regular(12.5), p.secondary);
    };
    if input.ask && !input.ai.review {
        title(ui, "Ask the agent");
        body(
            ui,
            &format!(
                "{} edits this project with Neptune Video's own tools while you watch: it can \
                 look at frames, import media, cut and arrange clips, add effects, \
                 titles and captions, transcribe speech, mix sound and export. Each \
                 edit appears here as it is made and is one step you can undo.",
                input.ai.provider.name()
            ),
        );
        ui.add_space(8.0);
        body(
            ui,
            "It sees the frames, transcripts and sound levels it asks for, and \
             the files in this project's folders, in your Movies, Music, \
             Pictures, Desktop, Documents and Downloads folders and in folders \
             you add in Preferences. It can also make, open and export other \
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
        ui.add_space(8.0);
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
        ui.add_space(12.0);
        examples(ui, p);
    }
}

/// The command language by example, as a card: the command and what it does
/// on each row.
fn examples(ui: &mut Ui, p: &Palette) {
    controls::group(ui, *p, |ui, group| {
        for (command, meaning) in EXAMPLES {
            let width = ui.available_width() - 28.0;
            let painter = ui.painter();
            let command = painter.layout(command.to_owned(), theme::mono(12.0), p.fg, width);
            let meaning =
                painter.layout(meaning.to_owned(), theme::regular(12.0), p.secondary, width);
            let height = (command.size().y + 2.0 + meaning.size().y + 14.0).max(32.0);
            // The row's own place among the panel's widgets, which scrolling
            // does not change.
            let id = ui.auto_id_with("agent-example");
            let rect = group.custom(ui, height);
            let top = rect.center().y - (command.size().y + 2.0 + meaning.size().y) * 0.5;
            let below = top + command.size().y + 2.0;
            // The command and what it does are read as two labels, as they
            // were before the card.
            for (salt, at, galley, ink) in [
                ("command", top, command, p.fg),
                ("meaning", below, meaning, p.secondary),
            ] {
                let shown = Rect::from_min_size(pos2(rect.left() + 14.0, at), galley.size());
                let text = galley.text().to_owned();
                ui.painter().galley(shown.min, galley, ink);
                ui.interact(shown, id.with(salt), Sense::hover())
                    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
            }
        }
        card_note(
            ui,
            group,
            "A line that starts with { is applied as a JSON command.",
        );
    });
}

/// A card's note, read as a label as well as painted.
fn card_note(ui: &mut Ui, group: &mut controls::Group, text: &str) {
    let id = ui.auto_id_with("agent-note");
    let top = ui.cursor().top();
    group.note(ui, text);
    let rect = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
    ui.interact(rect, id, Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
}

/// Words set in the conversation's column, wrapped to it and read out whole.
fn words(ui: &mut Ui, text: &str, font: FontId, ink: Color32) -> Response {
    let width = ui.available_width();
    let galley = ui.painter().layout(
        text.to_owned(),
        font,
        ink,
        (width - 2.0 * TEXT_INSET).max(40.0),
    );
    let (rect, response) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    ui.painter()
        .galley(pos2(rect.left() + TEXT_INSET, rect.top()), galley, ink);
    response
}

/// What leads a line of the conversation.
#[derive(Clone, Copy)]
enum Mark {
    Glyph(Icon, Color32),
    /// A step that is still running: a quiet dot, which needs no frames.
    Dot(Color32),
}

/// A line led by a 12-point mark in the column the session's dot stands in,
/// at least `row` tall, the mark level with its first line.
fn marked(ui: &mut Ui, mark: Mark, mut job: LayoutJob, row: f32) -> Response {
    let width = ui.available_width();
    let font = (job.sections.first())
        .map(|section| section.format.font_id.clone())
        .unwrap_or_else(|| theme::regular(13.0));
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    job.wrap.max_width = (width - MARKED - TEXT_INSET).max(40.0);
    let galley = ui.painter().layout_job(job);
    let pad = ((row - line) * 0.5).max(0.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y + 2.0 * pad), Sense::hover());
    let text = galley.text().to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    let painter = ui.painter();
    let centre = pos2(rect.left() + MARK_CENTRE, rect.top() + pad + line * 0.5);
    match mark {
        Mark::Glyph(icon, ink) => {
            icons::paint(
                painter,
                Rect::from_center_size(centre, Vec2::splat(12.0)),
                icon,
                ink,
            );
        }
        Mark::Dot(ink) => {
            painter.circle_filled(centre, 3.0, ink);
        }
    }
    painter.galley(
        pos2(rect.left() + MARKED, rect.top() + pad),
        galley,
        Color32::PLACEHOLDER,
    );
    response
}

/// The person's line: a bubble at the trailing edge, no wider than its words
/// need.
fn bubble(ui: &mut Ui, p: &Palette, text: &str, font: FontId) {
    let width = ui.available_width();
    let room = (width * BUBBLE - 20.0).max(40.0);
    let galley = ui.painter().layout(text.to_owned(), font, p.fg, room);
    // Terminal's bubble: a row's corner, and 10 by 7 round its words.
    let size = galley.size() + vec2(20.0, 14.0);
    let (row, response) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
    let rect = Rect::from_min_size(pos2(row.right() - size.x, row.top()), size);
    ui.interact(rect, response.id.with("said"), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    ui.painter()
        .rect_filled(rect, metrics::ROW_RADIUS, p.control);
    ui.painter()
        .galley(rect.min + vec2(10.0, 7.0), galley, p.fg);
}

/// What came of a line: one row per action, each with its mark and words.
fn answer(ui: &mut Ui, p: &Palette, ok: bool, lines: &[String]) {
    let (mark, ink) = if ok {
        (Mark::Glyph(Icon::CircleCheck, p.green), p.fg)
    } else {
        (Mark::Glyph(Icon::AlertCircle, p.red), p.secondary)
    };
    for line in lines {
        marked(ui, mark, plain(line, theme::regular(13.0), ink), 24.0);
    }
}

/// A request from a connected agent: a quiet line under its mark.
fn external(ui: &mut Ui, p: &Palette, ok: bool, text: &str) {
    let (mark, ink) = if ok {
        (Mark::Glyph(Icon::Sparkles, p.secondary), p.secondary)
    } else {
        (Mark::Glyph(Icon::AlertCircle, p.red), p.red)
    };
    marked(ui, mark, plain(text, theme::regular(12.0), ink), 24.0);
}

fn plain(text: &str, font: FontId, ink: Color32) -> LayoutJob {
    LayoutJob::simple(text.to_owned(), font, ink, f32::INFINITY)
}

/// The agent's words with the little Markdown models write: **strong**,
/// `code`, headings and bullets. Anything else is shown as it was written.
fn written(text: &str, p: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = egui::TextFormat::simple(theme::regular(13.0), p.fg);
    let strong = egui::TextFormat::simple(theme::medium(13.0), p.fg);
    let code = egui::TextFormat::simple(theme::mono(12.0), p.secondary);
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
    // The edit a step made is said after it, and why one was refused is the
    // one line that explains a failure: both are read as the step is.
    let (mark, detail, ink) = match &step.state {
        StepState::Running => (Mark::Dot(p.muted), None, p.secondary),
        StepState::Done(edit) => (
            Mark::Glyph(Icon::CircleCheck, p.green),
            edit.as_deref(),
            p.secondary,
        ),
        StepState::Refused(why) => (
            Mark::Glyph(Icon::AlertCircle, p.red),
            why.as_deref(),
            p.secondary,
        ),
        StepState::Stopped => (Mark::Glyph(Icon::Close, p.muted), None, p.muted),
    };
    let mut text = plain(&step.what, theme::regular(12.0), ink);
    if let Some(detail) = detail {
        text.append(
            &format!("  {detail}"),
            0.0,
            egui::TextFormat::simple(theme::regular(12.0), p.secondary),
        );
    }
    marked(ui, mark, text, 24.0);
}

/// The request in flight: that it is, for how long, and the way to stop it.
fn waiting_row(ui: &mut Ui, p: &Palette, waiting: &WaitingInput, actions: &mut Vec<Action>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, metrics::CONTROL_HEIGHT), Sense::hover());
    let line = rect.center().y;
    kit::spinner(
        ui,
        Rect::from_center_size(pos2(rect.left() + MARK_CENTRE, line), Vec2::splat(12.0)),
        p.muted,
    );
    let text = if waiting.cancelling {
        "Cancelling…".to_owned()
    } else if !waiting.started {
        format!("Starting {}…", waiting.provider)
    } else if waiting.working {
        // The picker names the provider; the time and the count are what
        // must fit beside Cancel.
        match waiting.edits {
            0 => format!("Working… {}s", waiting.seconds),
            1 => format!("Working… {}s · 1 change", waiting.seconds),
            n => format!("Working… {}s · {n} changes", waiting.seconds),
        }
    } else {
        format!("{} is planning… {}s", waiting.provider, waiting.seconds)
    };
    // Set in from the conversation's clip, so that the keyboard's ring
    // around Cancel is drawn whole.
    let cancel = place(
        ui,
        rect.shrink2(vec2(TEXT_INSET, 0.0)),
        Layout::right_to_left(Align::Center),
        "agent-cancel",
        |ui| {
            ui.add_enabled_ui(!waiting.cancelling, |ui| {
                controls::button_sized(ui, *p, "Cancel", ButtonKind::Secondary, ButtonSize::Compact)
            })
            .inner
        },
    );
    let room = cancel.rect.left() - 8.0 - (rect.left() + MARKED);
    let galley = elided(ui.painter(), &text, theme::regular(12.5), p.secondary, room);
    let said = galley_at(ui.painter(), pos2(rect.left() + MARKED, line), galley);
    ui.interact(said, ui.id().with("agent-waiting"), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    if cancel.clicked() {
        actions.push(Action::AiCancel);
    }
    // The seconds are read off a clock; the spinner alone would not say so.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
}

/// The glyph of an edit, read from the words the history names it with.
fn kind_of(step: &str) -> Icon {
    let step = step.to_lowercase();
    let first = step.split_whitespace().next().unwrap_or_default();
    if step.contains("delete") || first == "remove" {
        Icon::Trash
    } else if matches!(first, "split" | "trim") || step.contains("trim") {
        Icon::Scissors
    } else if matches!(first, "add" | "import" | "create") {
        Icon::Plus
    } else if matches!(first, "move" | "reorder") {
        Icon::ArrowRight
    } else {
        Icon::Pencil
    }
}

/// A row of a card led by a 14-point glyph, as tall as its words need and
/// `least` at least.
fn card_row(
    ui: &mut Ui,
    group: &mut controls::Group,
    glyph: Option<(Icon, Color32)>,
    job: LayoutJob,
    least: f32,
) {
    let mut job = job;
    let lead = if glyph.is_some() { 36.0 } else { 14.0 };
    job.wrap.max_width = (ui.available_width() - lead - 14.0).max(40.0);
    let galley = ui.painter().layout_job(job);
    // The row's own place among the panel's widgets, which scrolling does
    // not change.
    let id = ui.auto_id_with("agent-card-row");
    let rect = group.custom(ui, (galley.size().y + 14.0).max(least));
    let top = rect.center().y - galley.size().y * 0.5;
    if let Some((icon, ink)) = glyph {
        let first = (galley.rows.first()).map_or(galley.size().y, |row| row.rect().height());
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(rect.left() + 21.0, top + first * 0.5),
                Vec2::splat(14.0),
            ),
            icon,
            ink,
        );
    }
    let text = galley.text().to_owned();
    ui.painter()
        .galley(pos2(rect.left() + lead, top), galley, Color32::PLACEHOLDER);
    ui.interact(rect, id, Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
}

/// A proposal: what the model says it would do, the commands that do it,
/// what they change, and the decision.
fn plan(ui: &mut Ui, p: &Palette, card: &PlanCard, current: bool, actions: &mut Vec<Action>) {
    let plan = &card.plan;
    ui.push_id(plan.id, |ui| {
        controls::group(ui, *p, |ui, group| {
            // Who planned it, and where it stands.
            let rect = group.custom(ui, 32.0);
            let word = card.state.word();
            let word =
                ui.painter()
                    .layout_no_wrap(word.to_owned(), theme::regular(11.5), p.secondary);
            let word_width = word.size().x;
            galley_at(
                ui.painter(),
                pos2(rect.right() - 14.0 - word_width, rect.center().y),
                word,
            );
            let by = elided(
                ui.painter(),
                &plan.by,
                theme::medium(12.5),
                p.fg,
                rect.width() - 28.0 - word_width - 8.0,
            );
            galley_at(ui.painter(), pos2(rect.left() + 14.0, rect.center().y), by);
            ui.interact(rect, ui.id().with("plan-head"), Sense::hover())
                .widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Label,
                        true,
                        format!("{} · {}", plan.by, card.state.word()),
                    )
                });
            if !plan.summary.is_empty() {
                card_row(
                    ui,
                    group,
                    None,
                    plain(&plan.summary, theme::regular(12.5), p.fg),
                    32.0,
                );
            }
            for step in plan.steps.iter().take(STEPS_SHOWN) {
                card_row(
                    ui,
                    group,
                    Some((kind_of(step), p.secondary)),
                    plain(step, theme::regular(12.5), p.fg),
                    32.0,
                );
            }
            if plan.steps.len() > STEPS_SHOWN {
                card_row(
                    ui,
                    group,
                    None,
                    plain(
                        &format!("and {} more", plan.steps.len() - STEPS_SHOWN),
                        theme::regular(12.5),
                        p.secondary,
                    ),
                    32.0,
                );
            }
            if !plan.changes.is_empty() {
                card_note(ui, group, &plan.changes.join("  ·  "));
            }
            let note = |ui: &mut Ui, group: &mut controls::Group, glyph, text: &str| {
                card_row(
                    ui,
                    group,
                    Some(glyph),
                    plain(text, theme::regular(11.5), p.secondary),
                    32.0,
                );
            };
            match &card.state {
                PlanState::Open => {
                    if current {
                        card_note(ui, group, "Make these edits as one step you can undo");
                    } else {
                        note(
                            ui,
                            group,
                            (Icon::Warning, p.legible(p.attention)),
                            "The project changed since this was planned. Ask again.",
                        );
                    }
                    let rect = group.custom(ui, 46.0);
                    place(
                        ui,
                        rect.shrink2(vec2(12.0, 0.0)),
                        Layout::right_to_left(Align::Center),
                        "plan-decision",
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let apply = ui
                                .add_enabled_ui(current, |ui| {
                                    controls::button(ui, *p, "Apply", ButtonKind::Primary)
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
                                controls::button(ui, *p, "Discard", ButtonKind::Secondary)
                                    .on_hover_text("Close this plan; nothing is edited");
                            if discard.clicked() {
                                actions.push(Action::AiDiscard(plan.id));
                            }
                        },
                    );
                }
                state @ (PlanState::Empty | PlanState::Unchanged) => {
                    note(ui, group, (Icon::CircleCheck, p.muted), state.said());
                }
                state @ PlanState::Applied => {
                    note(ui, group, (Icon::CircleCheck, p.green), state.said());
                }
                state @ PlanState::Discarded => {
                    note(ui, group, (Icon::Close, p.muted), state.said());
                }
                state @ PlanState::Closed(_) => {
                    note(ui, group, (Icon::AlertCircle, p.red), state.said());
                }
            }
        });
    });
}

// ---- The composer ---------------------------------------------------------

/// The one surface at the panel's foot: the field, and under it what the
/// field does, who answers, and the round button that sends. A press on the
/// surface outside its controls writes in the field.
fn composer_surface(
    ui: &mut Ui,
    rect: Rect,
    state: &mut AgentState,
    input: &AgentInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let id = Id::new("agent-composer");
    let field = viewport_id(ui.ctx(), id);
    controls::field_surface(ui, *p, rect, id, (true, RADIUS));
    // Under every control: it takes only what they leave, and is not a stop
    // of its own for the keyboard.
    let surface = ui.interact(rect, id.with("surface"), Sense::CLICK);
    if surface.clicked() {
        ui.memory_mut(|memory| memory.request_focus(field));
    }
    surface.on_hover_cursor(CursorIcon::Text);

    // The field: a request to the model, or a line of the command language.
    // Enter sends and Shift+Enter starts a new line, as in Terminal's chat.
    let line = rect.bottom() - INSET - ROUND * 0.5;
    let words = Rect::from_min_max(
        pos2(rect.left() + SIDE, rect.top() + TOP),
        pos2(rect.right() - SIDE, line - FOOT * 0.5 - GAP),
    );
    let composing =
        ui.input(|input| (input.events.iter()).any(|event| matches!(event, egui::Event::Ime(_))));
    // Before the field runs: it would otherwise do nothing with the key, and
    // the key must not reach anything behind it.
    let entered = ui.memory(|memory| memory.has_focus(field)) && !composing && take_enter(ui);
    let hint = if input.ask && input.talking {
        "Reply, or ask for more"
    } else if input.ask {
        "Describe an edit"
    } else {
        "Type a command, or “help”"
    };
    let font = field_font(input.ask);
    let response = place(
        ui,
        words,
        Layout::top_down(Align::Min),
        "agent-field",
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("agent-field-scroll")
                .max_height(words.height())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut state.input)
                            .id(field)
                            // egui paints a field's hint in its weak text,
                            // `fg` at 60 %, whatever colour it is given: as
                            // Terminal's composer shows it.
                            .hint_text(egui::RichText::new(hint).font(font.clone()))
                            .font(font)
                            .frame(egui::Frame::NONE)
                            .background_color(Color32::TRANSPARENT)
                            .margin(egui::Margin::ZERO)
                            .desired_width(words.width())
                            .desired_rows(1)
                            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)),
                    )
                })
                .inner
        },
    );
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::TextEdit,
            ui.is_enabled(),
            if input.ask { "Request" } else { "Command line" },
        )
    });
    // A change of mode asks for the field, but one made with the arrows
    // leaves the keyboard on the choice, where the next arrow is read.
    let keyed = std::mem::take(&mut state.keyed);
    if std::mem::take(&mut state.focus) && !keyed {
        response.request_focus();
    }

    let send = Rect::from_center_size(
        pos2(rect.right() - INSET - ROUND * 0.5, line),
        Vec2::splat(ROUND),
    );
    let row = Rect::from_min_max(
        pos2(rect.left() + INSET, line - FOOT * 0.5),
        pos2(send.left() - 4.0, line + FOOT * 0.5),
    );
    state.keyed = foot(ui, row, input, actions);

    // One request at a time: while the model is asked, the button stops it.
    let stop = input.ask && input.waiting.is_some();
    let stopping = input.waiting.as_ref().is_some_and(|w| w.cancelling);
    let ready = if stop {
        !stopping
    } else {
        !state.input.trim().is_empty()
    };
    let name = if stop {
        "Stop the request"
    } else if input.ask {
        "Ask (Enter)"
    } else {
        "Run command (Enter)"
    };
    let clicked = send_button(ui, p, send, (stop, ready), name).clicked();
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
        // The keyboard stays in the field for the next line.
        state.focus = true;
    }
}

/// A fresh press of Enter without a modifier, taken from this frame's input
/// so the field does not see it.
// Neptune Video: Terminal's `chat::take_enter`.
fn take_enter(ui: &Ui) -> bool {
    ui.input_mut(|input| {
        let before = input.events.len();
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.shift && !modifiers.alt && !modifiers.ctrl && !modifiers.mac_cmd
            )
        });
        input.events.len() != before
    })
}

/// The row at the surface's foot: Ask or Command, and in Ask who answers.
/// Returns whether the arrows changed the choice.
fn foot(ui: &mut Ui, row: Rect, input: &AgentInput, actions: &mut Vec<Action>) -> bool {
    let p = input.p;
    // The switch gives "Command" room round it where the row also holds the
    // whole of who answers. Where it does not, the switch narrows as far as
    // its labels' fit to keep the whole of it, and where even that is not
    // enough it keeps its own width and who answers gives way. Its width is
    // the same in either mode.
    let described = input.ai.describe();
    let wide = (row.width() - 4.0 - pill_width(ui, &described)).floor();
    let fit = controls::segmented_fit(ui, &["Ask", "Command"]).ceil();
    let switch = if wide >= fit.min(MODE) {
        wide.min(ROOMY_MODE)
    } else {
        MODE
    };
    let mode = Rect::from_min_size(
        pos2(row.left(), row.center().y - 14.0),
        vec2(switch.min(row.width()), 28.0),
    );
    let mut keyed = false;
    place(
        ui,
        mode,
        Layout::left_to_right(Align::Center),
        "agent-mode",
        |ui| {
            let name = "Ask|Command";
            let base = ui.id().with(("segmented", name));
            let cells = [base.with("Ask"), base.with("Command")];
            let mut ask = input.ask;
            // Left and Right move the choice, as in Terminal's strips.
            if let Some(to) = super::arrow_choice(ui.ctx(), &cells, usize::from(!ask)) {
                ask = to == 0;
                keyed = true;
            }
            controls::segmented(
                ui,
                *p,
                name,
                &mut ask,
                &[(true, "Ask"), (false, "Command")],
                mode.width(),
            );
            if ask != input.ask {
                actions.push(Action::SetAgentAsk(ask));
            }
        },
    );
    if input.ask {
        let room = Rect::from_min_max(pos2(mode.right() + 4.0, row.top()), row.max);
        model(ui, room, input, &described, actions);
    }
    keyed
}

/// The width of the pill that shows `words`.
fn pill_width(ui: &Ui, words: &str) -> f32 {
    (ui.painter())
        .layout_no_wrap(words.to_owned(), theme::medium(12.0), Color32::PLACEHOLDER)
        .size()
        .x
        + 2.0 * PAD
        + CHEVRON
}

/// The fewest points of words a description cut short keeps past the
/// provider's name: fewer say nothing.
const LEAST: f32 = 28.0;

/// Who answers, as a pill that opens the choices: the provider, the effort
/// and how edits are made, and the way to the model's own preferences.
fn model(ui: &mut Ui, room: Rect, input: &AgentInput, described: &str, actions: &mut Vec<Action>) {
    let p = input.p;
    let provider = input.ai.provider.name();
    // The whole description where it fits. Where it does not, as much of it
    // as the room holds, cut short, while that keeps more than the
    // provider's name; then the provider alone, and in the narrowest panel
    // only its chevron. The name and the tooltip always say all of it.
    let (width, shown) = if pill_width(ui, described) <= room.width() {
        (pill_width(ui, described), Some(described))
    } else if pill_width(ui, provider) + LEAST <= room.width() {
        (room.width(), Some(described))
    } else if pill_width(ui, provider) <= room.width() {
        (pill_width(ui, provider), Some(provider))
    } else {
        (26.0, None)
    };
    let rect = Rect::from_min_size(pos2(room.left(), room.center().y - 13.0), vec2(width, 26.0));
    if rect.width() > room.width() {
        return;
    }
    let response = pill(
        ui,
        p,
        rect,
        Id::new("agent-model"),
        shown,
        &format!("Model: {described}"),
    );
    // Its list opens over the composer, where the panel has room, its
    // trailing edge on the pill's so it stays in the window, and scrolls in
    // a window too short for it.
    let above = (rect.top() - ui.ctx().content_rect().top() - 8.0 - 12.0).max(120.0);
    egui::Popup::menu(&response)
        .align(RectAlign::TOP_END)
        .align_alternatives(&[])
        .show(|ui| {
            menu_style(ui, 210.0);
            let list = egui::ScrollArea::vertical()
                .id_salt("agent-model-list")
                .max_height(above)
                .show(ui, |ui| {
                    let choice = |ui: &mut Ui, on: bool, name: &str| {
                        kit::menu_row(ui, p, None, name, "", false, Some(on)).clicked()
                    };
                    kit::menu_heading(ui, p, "Answered by");
                    for provider in Provider::ALL {
                        if choice(ui, input.ai.provider == provider, provider.name()) {
                            actions.push(Action::SetAiProvider(provider));
                            ui.close();
                        }
                    }
                    widgets::menu_separator(ui, p);
                    kit::menu_heading(ui, p, "Effort");
                    for effort in Effort::ALL {
                        if choice(ui, input.ai.effort == effort, effort.name()) {
                            actions.push(Action::SetAiEffort(effort));
                            ui.close();
                        }
                    }
                    widgets::menu_separator(ui, p);
                    kit::menu_heading(ui, p, "Edits");
                    for (review, name) in
                        [(false, "Made as it works"), (true, "Proposed for review")]
                    {
                        if choice(ui, input.ai.review == review, name) {
                            actions.push(Action::SetAiReview(review));
                            ui.close();
                        }
                    }
                    widgets::menu_separator(ui, p);
                    if menu_item(ui, p, Some(Icon::Settings), "Model…", "").clicked() {
                        actions.push(Action::OpenPreferences(PreferencesPane::Agent));
                        ui.close();
                    }
                });
            // A list cut short by the window says it goes on.
            kit::scroll_fades(ui, &list, p.elevated);
        });
    kit::rich_tooltip(
        response,
        p,
        described,
        &["Who answers. It uses the sign-in of the Claude or Codex command line on this computer."],
        None,
    );
}

/// A control of the composer's row: words and a chevron that open a list,
/// resting in the secondary ink and lit under the pointer and while its list
/// is open. `name` is its accessible name.
// Neptune Video: Terminal's composer `pill` (`T:src/ui/project/composer.rs`)
// without a CLI's mark. The composer's row gives it a rect, which its words
// are elided in.
fn pill(ui: &mut Ui, p: &Palette, rect: Rect, id: Id, words: Option<&str>, name: &str) -> Response {
    let id = viewport_id(ui.ctx(), id);
    let live = ui.is_enabled();
    let response = ui.interact(rect, id, if live { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, live, name));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let lit = live && (response.hovered() || response.has_focus() || open);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if live && (open || response.is_pointer_button_down_on()) {
            painter.rect_filled(rect, PILL_RADIUS, p.pressed);
        } else if live && response.hovered() {
            painter.rect_filled(rect, PILL_RADIUS, p.hover);
        }
        if response.has_focus() {
            focus_ring(painter, rect, PILL_RADIUS, *p);
        }
        let ink = if lit { p.fg } else { p.secondary };
        let chevron = match words {
            Some(words) => {
                let x = rect.left() + PAD;
                let room = rect.right() - PAD - CHEVRON - x;
                let shown = galley_at(
                    painter,
                    Pos2::new(x, rect.center().y),
                    elided(painter, words, theme::medium(12.0), ink, room.max(0.0)),
                );
                shown.right() + 4.0 + 5.0
            }
            None => rect.center().x,
        };
        icons::paint(
            painter,
            Rect::from_center_size(Pos2::new(chevron, rect.center().y + 0.5), Vec2::splat(10.0)),
            Icon::ChevronDown,
            if lit { p.fg } else { p.muted },
        );
    }
    if live {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The control at the composer's trailing edge: it sends, or stops the
/// request that is being answered. Round, as the one control of the composer
/// that acts; in the accent only while there is something to send.
// Neptune Video: Terminal's `chat::send_button_named`, with the stop glyph
// at 10 points and the name given for each state.
fn send_button(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    (stop, enabled): (bool, bool),
    name: &str,
) -> Response {
    let response = ui.interact(
        rect,
        ui.id().with("agent-send"),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = (rect.height() * 0.5).round() as u8;
        let fill = if enabled && !stop {
            p.accent
        } else {
            p.control
        };
        painter.rect_filled(rect, radius, fill);
        if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(rect, radius, p.pressed);
        } else if enabled && response.hovered() {
            painter.rect_filled(rect, radius, p.hover);
        }
        if response.has_focus() {
            focus_ring(painter, rect, radius, *p);
        }
        let (icon, size, ink) = if stop {
            (Icon::Stop, 10.0, if enabled { p.fg } else { p.muted })
        } else if enabled {
            (Icon::ArrowUp, 14.0, p.on_accent)
        } else {
            (Icon::ArrowUp, 14.0, p.muted)
        };
        icons::paint(
            painter,
            Rect::from_center_size(rect.center(), Vec2::splat(size)),
            icon,
            ink,
        );
        if stop {
            painter.rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
    }
    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let (shown, keys) = kit::name_and_keys(name);
    icons::hint(response, shown, keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Shape, accesskit::Role};
    use std::path::PathBuf;

    /// The window's 760 × 560 layout: the panel's body under its tabs.
    const NARROW: Vec2 = vec2(760.0, 560.0);
    const BODY: Rect = Rect::from_min_max(pos2(492.0, 78.0), pos2(752.0, 333.0));

    struct Seen {
        id: egui::accesskit::NodeId,
        name: String,
        rect: Rect,
        role: Role,
        disabled: bool,
    }

    /// The panel drawn in real egui passes in the body of a narrow window,
    /// its controls found by the names accessibility is given and pressed
    /// with the pointer.
    struct Bench {
        ctx: egui::Context,
        p: Palette,
        time: f64,
        state: AgentState,
        ai: AiSettings,
        session: Option<PathBuf>,
        problem: Option<String>,
        ask: bool,
        waiting: Option<bool>,
        talking: bool,
        current: bool,
        proposal_waits: bool,
        /// Whether Preferences is open over the panel.
        preferences: bool,
        /// The panel's body, in a window `NARROW` large.
        body: Rect,
        seen: Vec<Seen>,
        focused: Option<String>,
        /// The rectangles painted this frame, with their fills.
        fills: Vec<(Rect, Color32)>,
        /// The outlines painted this frame, with their colours.
        rings: Vec<(Rect, Color32)>,
        /// The gradients painted this frame, with the ground they fade into.
        fades: Vec<(Rect, Color32)>,
        painted: Vec<String>,
        copied: Vec<String>,
        actions: Vec<Action>,
    }

    impl Bench {
        fn new() -> Self {
            let ctx = egui::Context::default();
            let p = Palette::new(theme::Appearance::Dark, theme::Accent::default());
            theme::install_fonts(&ctx);
            theme::apply(&ctx, &p);
            ctx.enable_accesskit();
            Self {
                ctx,
                p,
                time: 0.0,
                state: AgentState::default(),
                ai: AiSettings::default(),
                session: Some(PathBuf::from("/tmp/sessions/window.json")),
                problem: None,
                ask: true,
                waiting: None,
                talking: false,
                current: true,
                proposal_waits: false,
                preferences: false,
                body: BODY,
                seen: Vec::new(),
                focused: None,
                fills: Vec::new(),
                rings: Vec::new(),
                fades: Vec::new(),
                painted: Vec::new(),
                copied: Vec::new(),
                actions: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, NARROW)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let input = AgentInput {
                p: &self.p,
                session: self.session.as_deref(),
                session_problem: self.problem.as_deref(),
                ask: self.ask,
                ai: &self.ai,
                waiting: self.waiting.map(|cancelling| WaitingInput {
                    provider: "Claude",
                    seconds: 3,
                    started: true,
                    cancelling,
                    working: true,
                    edits: 1,
                }),
                current: self.current,
                talking: self.talking,
                proposal_waits: self.proposal_waits,
            };
            let mut actions = Vec::new();
            let ctx = self.ctx.clone();
            let output = ctx.run_ui(raw, |ui| {
                show(ui, self.body, &mut self.state, &input, &mut actions);
                if self.preferences {
                    let settings = crate::settings::Settings::default();
                    let cache = crate::proxies::Cache::default();
                    crate::ui::sheets::settings(ui.ctx(), &self.p, &settings, cache, &mut actions);
                }
            });
            if let Some(update) = &output.platform_output.accesskit_update {
                let name = |node: &egui::accesskit::Node| {
                    node.label().or_else(|| node.value()).map(str::to_owned)
                };
                self.focused = (update.nodes.iter())
                    .find(|(id, _)| *id == update.focus)
                    .and_then(|(_, node)| name(node));
                self.seen = (update.nodes.iter())
                    .filter_map(|(id, node)| {
                        let bounds = node.bounds()?;
                        Some(Seen {
                            id: *id,
                            name: name(node)?,
                            rect: Rect::from_min_max(
                                pos2(bounds.x0 as f32, bounds.y0 as f32),
                                pos2(bounds.x1 as f32, bounds.y1 as f32),
                            ),
                            role: node.role(),
                            disabled: node.is_disabled(),
                        })
                    })
                    .collect();
            }
            self.fills.clear();
            self.rings.clear();
            self.fades.clear();
            self.painted.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, self);
            }
            self.copied.extend(
                (output.platform_output.commands.iter()).filter_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                }),
            );
            output.drop_without_applying_deltas();
            // As the app does: a change of mode asks for the field.
            for action in &actions {
                if let Action::SetAgentAsk(ask) = action {
                    self.ask = *ask;
                    self.state.focus = true;
                }
            }
            self.actions.extend(actions);
        }

        fn settle(&mut self) {
            for _ in 0..3 {
                self.frame(Vec::new());
            }
        }

        fn get(&self, name: &str, role: Role) -> &Seen {
            (self.seen.iter())
                .find(|seen| seen.name == name && seen.role == role)
                .unwrap_or_else(|| {
                    let names: Vec<_> = (self.seen.iter())
                        .map(|seen| format!("{:?} {}", seen.role, seen.name))
                        .collect();
                    panic!("FIXTURE: no {role:?} is named \"{name}\" among {names:?}")
                })
        }

        fn click(&mut self, name: &str, role: Role) {
            let at = self.get(name, role).rect.center();
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

        fn key(&mut self, key: Key) {
            self.chord(Modifiers::NONE, key);
        }

        fn chord(&mut self, modifiers: Modifiers, key: Key) {
            let event = |pressed| Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            };
            self.frame(vec![event(true), event(false)]);
        }

        fn type_text(&mut self, text: &str) {
            self.frame(vec![Event::Text(text.into())]);
        }

        /// The field's words: the composer's top less the room over them.
        fn field(&self) -> Rect {
            (self.seen.iter())
                .find(|seen| matches!(seen.role, Role::MultilineTextInput | Role::TextInput))
                .expect("FIXTURE: the field is there")
                .rect
        }

        /// Whether the conversation's lower edge is faded, or its upper.
        fn faded(&self, below: bool) -> bool {
            (self.fades.iter()).any(|(rect, ground)| {
                *ground == self.p.chrome
                    && (rect.height() - kit::SCROLL_FADE).abs() < 0.01
                    && (rect.center().y > BODY.center().y) == below
            })
        }

        fn filled(&self, color: Color32) -> bool {
            self.fills.iter().any(|(_, fill)| *fill == color)
        }
    }

    fn collect(shape: &Shape, bench: &mut Bench) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, bench);
                }
            }
            Shape::Rect(rect) => {
                bench.fills.push((rect.rect, rect.fill));
                if rect.stroke.width > 0.0 {
                    bench.rings.push((rect.rect, rect.stroke.color));
                }
            }
            Shape::Circle(circle) => bench.fills.push((
                Rect::from_center_size(circle.center, Vec2::splat(2.0 * circle.radius)),
                circle.fill,
            )),
            // A scroll fade: a band from its ground to nothing.
            Shape::Mesh(mesh) if mesh.vertices.len() == 4 => {
                let ground = (mesh.vertices.iter())
                    .map(|vertex| vertex.color)
                    .find(|color| color.a() == 255);
                if let Some(ground) = ground {
                    bench.fades.push((mesh.calc_bounds(), ground));
                }
            }
            // Words cut short are what is seen of them.
            Shape::Text(text) if text.galley.elided => bench.painted.push(
                (text.galley.rows.iter())
                    .flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr))
                    .collect(),
            ),
            Shape::Text(text) => bench.painted.push(text.galley.text().to_owned()),
            _ => {}
        }
    }

    // §5.6 and A3: the connect command leaves the panel for the session
    // row's "More", which copies it and says so.
    #[test]
    fn the_connect_command_is_copied_from_more_and_is_not_shown() {
        let mut bench = Bench::new();
        bench.settle();
        assert!(
            !(bench.painted.iter()).any(|text| text.contains("--connect")),
            "the command line is painted in the panel: {:?}",
            bench.painted
        );
        assert_eq!(
            bench.get("Agent session is open", Role::Label).role,
            Role::Label
        );
        bench.click("More", Role::Button);
        bench.click("Copy the connect command", Role::Button);
        assert_eq!(
            bench.copied,
            ["neptune-video-agent --connect /tmp/sessions/window.json"]
        );
        assert!(
            (bench.actions.iter()).any(
                |action| matches!(action, Action::Notify(text) if text == "Connect command copied")
            ),
            "{:?}",
            bench.actions.len()
        );

        // Without a session there is nothing to copy, and the row says why.
        let mut bench = Bench::new();
        bench.session = None;
        bench.problem = Some("The folder is read-only".into());
        bench.settle();
        bench.get("The session could not be opened.", Role::Label);
        bench.click("More", Role::Button);
        assert!(bench.get("Copy the connect command", Role::Button).disabled);
        // Why is read in the menu, not only under the pointer.
        bench.get("The folder is read-only", Role::Label);
        assert!(bench.copied.is_empty());
    }

    // §5.6 and A4: the session's dot says a proposal waits, from what the
    // planner says as the tab's count does, in the plain attention fill; a
    // failed session stays red.
    #[test]
    fn the_session_dot_says_a_proposal_waits_unless_the_session_failed() {
        let mut bench = Bench::new();
        bench.settle();
        assert!(bench.filled(bench.p.green));
        bench.proposal_waits = true;
        bench.settle();
        assert!(
            bench.filled(bench.p.attention),
            "the waiting dot is not attention"
        );
        assert!(!bench.filled(bench.p.green));

        bench.session = None;
        bench.problem = Some("The folder is read-only".into());
        bench.settle();
        assert!(bench.filled(bench.p.red), "the failure is not red");
        assert!(!bench.filled(bench.p.attention));
    }

    // §5.6: the round button is named for what it does in each state, and
    // cannot be pressed with nothing to send.
    #[test]
    fn the_send_button_is_named_for_what_it_does() {
        let mut bench = Bench::new();
        bench.settle();
        let send = bench.get("Ask (Enter)", Role::Button);
        assert!(send.disabled, "an empty field has nothing to ask");
        assert_eq!(send.rect.size(), Vec2::splat(ROUND));
        // §3.10: nothing in the panel takes the accent at rest.
        assert!(!bench.filled(bench.p.accent));

        bench.state.input = "Tighten the opening".into();
        bench.settle();
        assert!(!bench.get("Ask (Enter)", Role::Button).disabled);
        assert!(bench.filled(bench.p.accent), "the send is not lit");
        bench.click("Ask (Enter)", Role::Button);
        assert!(matches!(
            bench.actions.as_slice(),
            [Action::AiSubmit(text)] if text == "Tighten the opening"
        ));
        assert!(bench.state.input.is_empty());

        bench.ask = false;
        bench.settle();
        assert!(bench.get("Run command (Enter)", Role::Button).disabled);

        bench.ask = true;
        bench.waiting = Some(false);
        bench.settle();
        assert!(!bench.get("Stop the request", Role::Button).disabled);
        bench.waiting = Some(true);
        bench.settle();
        assert!(bench.get("Stop the request", Role::Button).disabled);
        assert!(bench.painted.iter().any(|text| text == "Cancelling…"));
    }

    // §3.10 and §9.12: the send lies on `hover` under the pointer and on
    // `pressed` while it is held, and rings with the keyboard.
    #[test]
    fn the_send_button_lights_under_the_pointer_and_rings_with_the_keyboard() {
        let mut bench = Bench::new();
        bench.state.input = "Tighten the opening".into();
        bench.settle();
        let send = bench.get("Ask (Enter)", Role::Button).rect;
        let near = |a: Rect, b: Rect| (a.min - b.min).length() + (a.max - b.max).length() < 0.1;
        let lies = |bench: &Bench, fill: Color32| {
            (bench.fills.iter()).any(|(rect, painted)| near(*rect, send) && *painted == fill)
        };
        let ringed = |bench: &Bench| {
            let ring = theme::tint(bench.p.accent, 0.55);
            (bench.rings.iter())
                .any(|(rect, color)| near(*rect, send.expand(2.0)) && *color == ring)
        };
        assert!(!lies(&bench, bench.p.hover) && !lies(&bench, bench.p.pressed));
        assert!(!ringed(&bench), "a ring at rest");
        bench.frame(vec![Event::PointerMoved(send.center())]);
        assert!(lies(&bench, bench.p.hover), "under the pointer");
        let button = |pressed| Event::PointerButton {
            pos: send.center(),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        bench.frame(vec![button(true)]);
        assert!(lies(&bench, bench.p.pressed), "held");
        // Let go beside it, so nothing is sent.
        let beside = send.left_center() - vec2(80.0, 0.0);
        bench.frame(vec![Event::PointerMoved(beside)]);
        bench.frame(vec![Event::PointerButton {
            pos: beside,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        bench.frame(vec![Event::PointerGone]);
        assert!(bench.actions.is_empty(), "{:?}", bench.actions);
        assert!(!lies(&bench, bench.p.hover) && !lies(&bench, bench.p.pressed));
        for _ in 0..8 {
            if bench.focused.as_deref() == Some("Ask (Enter)") {
                break;
            }
            bench.key(Key::Tab);
        }
        assert_eq!(bench.focused.as_deref(), Some("Ask (Enter)"));
        assert!(ringed(&bench), "with the keyboard");
    }

    // §5.6 and A2: at 760 × 560 the introduction keeps every word, runs on
    // under the composer behind a fade, and scrolls to its end.
    #[test]
    fn the_introduction_is_whole_and_scrolls_at_760_by_560() {
        for (ask, review, last) in [
            (true, false, "Follow-up requests continue the conversation."),
            (true, true, "Your media files are not sent."),
            (
                false,
                false,
                "A line that starts with { is applied as a JSON command.",
            ),
        ] {
            let mut bench = Bench::new();
            bench.ask = ask;
            bench.ai.review = review;
            bench.settle();
            // The paragraph, or the card's note, that ends it.
            let bottom = |bench: &Bench| {
                (bench.seen.iter())
                    .find(|seen| seen.role == Role::Label && seen.name.ends_with(last))
                    .unwrap_or_else(|| panic!("FIXTURE: nothing ends with \"{last}\""))
                    .rect
                    .bottom()
            };
            // A row of the examples keeps its node as the log scrolls.
            let example = |bench: &Bench| {
                (bench.seen.iter())
                    .find(|seen| seen.name == "split" && seen.role == Role::Label)
                    .map(|seen| seen.id)
            };
            let before = example(&bench);
            assert_eq!(before.is_some(), !ask);
            let log = bench.field().top() - TOP - INSET;
            assert!(bottom(&bench) > log, "the words fit; nothing is tested");
            assert!(bench.faded(true), "clipped words show no fade");
            assert!(!bench.faded(false), "the top is in view and faded");

            let over = pos2(BODY.center().x, BODY.top() + 60.0);
            bench.frame(vec![Event::PointerMoved(over)]);
            for _ in 0..8 {
                bench.frame(vec![Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -400.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                }]);
            }
            for _ in 0..20 {
                bench.frame(Vec::new());
            }
            assert!(
                bottom(&bench) <= log + 0.5,
                "the end of the introduction is at {} under the composer at {log}",
                bottom(&bench)
            );
            assert!(!bench.faded(true), "the end is in view and still faded");
            assert!(bench.faded(false), "the scrolled top shows no fade");
            if let Some(before) = before
                && let Some(after) = example(&bench)
            {
                assert_eq!(before, after, "the row's node changed as it scrolled");
            }
        }
    }

    // §5.6 and A1: the person's lines are bubbles in the control fill, not
    // an accent tint, at the trailing edge.
    #[test]
    fn the_persons_lines_are_control_bubbles_at_the_trailing_edge() {
        let mut bench = Bench::new();
        bench
            .state
            .push(Entry::Prompt("Tighten the opening".into()));
        bench.state.push(Entry::Asked("split".into()));
        bench.settle();
        assert!(bench.filled(bench.p.control));
        assert!(!bench.filled(theme::tint(bench.p.accent, 0.28)));
        for line in ["Tighten the opening", "split"] {
            let said = bench.get(line, Role::Label).rect;
            assert_eq!(
                said.right(),
                BODY.right(),
                "{line} is not at the trailing edge"
            );
            assert!(
                said.width() < BODY.width() * 0.6,
                "{line} is wider than its words: {said:?}"
            );
        }
    }

    // §5.6: a proposal is a card whose footer applies or discards it; one
    // planned for an earlier project cannot be applied and says so, and one
    // decided says what became of it.
    #[test]
    fn a_proposal_is_decided_from_its_card() {
        let id = uuid::Uuid::new_v4();
        let mut bench = Bench::new();
        bench.state.push(card(id));
        bench.settle();
        // The log scrolls its newest entry into view before it is pressed.
        for _ in 0..20 {
            bench.frame(Vec::new());
        }
        // What it changes, and what Apply does, are read before it is.
        bench.get("Clips 3 → 4", Role::Label);
        bench.get("Make these edits as one step you can undo", Role::Label);
        bench.click("Apply", Role::Button);
        bench.click("Discard", Role::Button);
        assert!(matches!(
            bench.actions.as_slice(),
            [Action::AiApply(a), Action::AiDiscard(b)] if *a == id && *b == id
        ));

        bench.current = false;
        bench.settle();
        assert!(bench.get("Apply", Role::Button).disabled);
        let stale = "The project changed since this was planned. Ask again.";
        assert!(bench.painted.iter().any(|text| text == stale));

        bench.state.settle(id, PlanState::Applied);
        bench.settle();
        assert!(!(bench.seen.iter()).any(|seen| seen.name == "Apply"));
        assert!((bench.painted.iter()).any(|text| text == PlanState::Applied.said()));
    }

    // §5.6 and §9.10: in the narrowest window's panel the composer's row
    // still holds Ask │ Command, who answers and the send, side by side.
    #[test]
    fn the_composers_row_keeps_its_controls_in_the_narrowest_panel() {
        let mut bench = Bench::new();
        bench.body = Rect::from_min_size(pos2(492.0, 78.0), vec2(226.0, 222.0));
        bench.settle();
        let ask = bench.get("Ask", Role::RadioButton).rect;
        let command = bench.get("Command", Role::RadioButton).rect;
        let named = format!("Model: {}", bench.ai.describe());
        let model = bench.get(&named, Role::ComboBox).rect;
        let send = bench.get("Ask (Enter)", Role::Button).rect;
        assert!(ask.right() <= command.left() + 0.5);
        assert!(command.right() < model.left(), "{command:?} {model:?}");
        assert!(model.right() < send.left(), "{model:?} {send:?}");
        assert!(send.right() <= bench.body.right() - INSET + 0.5);
        assert!(model.width() >= 22.0, "who answers is under 22 points");
    }

    // The composer's model and Preferences' are two controls: with the
    // sheet open on its Agent pane over the panel, as "Model…" opens it,
    // neither takes the other's place.
    #[test]
    fn preferences_open_over_the_panel_keeps_its_model_apart() {
        let mut bench = Bench::new();
        bench.preferences = true;
        PreferencesPane::Agent.show(&bench.ctx);
        bench.settle();
        let named = format!("Model: {}", bench.ai.describe());
        let pill = bench.get(&named, Role::ComboBox).id;
        let field = bench.get("Model", Role::TextInput).id;
        assert_ne!(pill, field);
    }

    // §7.3: Left and Right move the choice of Ask or Command.
    #[test]
    fn the_arrows_choose_between_ask_and_command() {
        let mut bench = Bench::new();
        bench.settle();
        for _ in 0..6 {
            if bench.focused.as_deref() == Some("Ask") {
                break;
            }
            bench.key(Key::Tab);
        }
        assert_eq!(bench.focused.as_deref(), Some("Ask"));
        // The app gives the field the keyboard as the mode changes; the
        // arrows keep it on the choice, where the next arrow is read.
        bench.key(Key::ArrowRight);
        bench.settle();
        assert_eq!(bench.focused.as_deref(), Some("Command"));
        bench.key(Key::ArrowLeft);
        bench.settle();
        assert_eq!(bench.focused.as_deref(), Some("Ask"));
        assert!(matches!(
            bench.actions.as_slice(),
            [Action::SetAgentAsk(false), Action::SetAgentAsk(true)]
        ));

        // A press on the choice writes in the field, named for what it
        // takes.
        bench.click("Command", Role::RadioButton);
        bench.settle();
        assert_eq!(bench.focused.as_deref(), Some("Command line"));
    }

    // §5.6: Enter sends and Shift+Enter starts a new line; while the model
    // is asked, Enter keeps the words for later.
    #[test]
    fn enter_sends_and_shift_enter_starts_a_line() {
        let mut bench = Bench::new();
        bench.ask = false;
        bench.state.focus = true;
        bench.settle();
        assert_eq!(bench.focused.as_deref(), Some("Command line"));
        // An empty field sends nothing.
        bench.key(Key::Enter);
        assert!(bench.actions.is_empty());
        assert!(bench.state.input.is_empty());

        bench.type_text("split");
        bench.chord(Modifiers::SHIFT, Key::Enter);
        bench.type_text("undo");
        assert_eq!(bench.state.input, "split\nundo");
        assert!(bench.actions.is_empty());
        bench.key(Key::Enter);
        assert!(matches!(
            bench.actions.as_slice(),
            [Action::AgentSubmit(text)] if text == "split\nundo"
        ));
        assert!(bench.state.input.is_empty());
        bench.settle();
        assert_eq!(bench.focused.as_deref(), Some("Command line"));

        let mut bench = Bench::new();
        bench.waiting = Some(false);
        bench.state.focus = true;
        bench.settle();
        bench.type_text("Now the ending");
        bench.key(Key::Enter);
        assert!(bench.actions.is_empty(), "a second request was sent");
        assert_eq!(bench.state.input, "Now the ending");
    }

    // §4.1 and §5.6: who answers keeps the effort in view at the default
    // window, the switch narrowing toward its labels' fit to leave it room,
    // and widens only where the whole description still fits beside it.
    #[test]
    fn who_answers_keeps_the_effort_in_view() {
        let described = AiSettings::default().describe();
        assert_eq!(described, "Claude · High");
        let pill = |bench: &Bench| {
            (bench.painted.iter())
                .find(|text| text.starts_with("Claude") && text.len() < 20)
                .cloned()
                .expect("FIXTURE: the pill is painted")
        };
        let switch = |bench: &Bench| {
            bench.get("Ask", Role::RadioButton).rect.width()
                + bench.get("Command", Role::RadioButton).rect.width()
                + 4.0
        };
        // The 1440 × 900 window's panel.
        let mut bench = Bench::new();
        bench.body = Rect::from_min_size(pos2(400.0, 78.0), vec2(286.0, 255.0));
        bench.settle();
        assert_eq!(pill(&bench), described);
        // Narrower than its own width, with room still round "Command".
        assert!(switch(&bench) < MODE, "{}", switch(&bench));
        let ctx = bench.ctx.clone();
        let command = ctx.fonts_mut(|fonts| {
            (fonts.layout_no_wrap("Command".to_owned(), theme::medium(11.5), Color32::WHITE))
                .size()
                .x
        });
        let cell = bench.get("Command", Role::RadioButton).rect.width();
        assert!(cell >= command + 16.0 - 0.01, "{cell} for {command}");
        // A wider panel.
        bench.body = Rect::from_min_size(pos2(300.0, 78.0), vec2(400.0, 255.0));
        bench.settle();
        assert_eq!(pill(&bench), described);
        assert_eq!(switch(&bench), ROOMY_MODE);
        // The 760 × 560 window's panel: the provider alone.
        bench.body = BODY;
        bench.settle();
        assert_eq!(pill(&bench), "Claude");
        assert_eq!(switch(&bench), MODE);
        bench.get(&format!("Model: {described}"), Role::ComboBox);
    }

    // §7.4: Cancel stands in from the conversation's clip, so that its
    // keyboard ring is whole.
    #[test]
    fn the_waiting_rows_cancel_stands_in_from_the_clip() {
        let mut bench = Bench::new();
        bench.ask = false;
        bench.waiting = Some(false);
        bench.state.push(Entry::Asked("split".into()));
        bench.settle();
        let cancel = bench.get("Cancel", Role::Button).rect;
        assert!(
            cancel.right() <= BODY.right() - 3.0 - 2.0,
            "{cancel:?} in {BODY:?}"
        );
    }

    fn card(id: uuid::Uuid) -> Entry {
        Entry::Plan(PlanCard {
            plan: Plan {
                id,
                summary: "Tighten the opening".into(),
                steps: vec!["Trim clip".into()],
                changes: vec!["Clips 3 → 4".into()],
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
