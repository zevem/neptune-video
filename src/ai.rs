//! The model behind the agent panel.
//!
//! A request goes to the Claude or Codex command line the person has already
//! signed in to. It is answered one of two ways:
//!
//! - **As it works** (`Agent`): the provider edits this window's project
//!   with the tools of `neptune-video-mcp`, the same tools an outside agent has,
//!   started in this window. Every edit lands here at once, through the same
//!   guarded requests an outside agent sends, as one step of the history.
//!   `editor_agent::Conversation` owns the provider's process.
//! - **For review** (`Planner`): the provider returns a proposal of
//!   ordinary commands, through `editor_agent::AiRuntime`, validated against
//!   the document it was planned for; it changes nothing until the person
//!   applies it.
//!
//! This module is the only place the application touches either runtime.
//! Nothing here waits for a provider, and an answer a runtime did not give is
//! never made up: every state the panel shows follows from an event.
use editor_agent::{
    AiConfig, AiContext, AiEvent, AiProposal, AiProvider, AiRuntime, Conversation, ToolServer,
    TurnEvent,
};
use editor_core::{Clip, ClipId, Command, Editor, Project, Time};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Steps listed on a proposal before the rest are counted.
pub const STEPS_SHOWN: usize = 6;
const MODEL_LIMIT: usize = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
        }
    }

    fn runtime(self) -> AiProvider {
        match self {
            Provider::Claude => AiProvider::Claude,
            Provider::Codex => AiProvider::Codex,
        }
    }

    fn of(provider: &AiProvider) -> Self {
        match provider {
            AiProvider::Claude => Provider::Claude,
            AiProvider::Codex => Provider::Codex,
        }
    }
}

/// How hard the model is asked to think. `Default` leaves it to the provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Default,
    Low,
    Medium,
    #[default]
    High,
}

impl Effort {
    pub const ALL: [Effort; 4] = [Effort::Default, Effort::Low, Effort::Medium, Effort::High];

    pub fn name(self) -> &'static str {
        match self {
            Effort::Default => "Default",
            Effort::Low => "Low",
            Effort::Medium => "Medium",
            Effort::High => "High",
        }
    }

    fn value(self) -> Option<String> {
        match self {
            Effort::Default => None,
            Effort::Low => Some("low".to_owned()),
            Effort::Medium => Some("medium".to_owned()),
            Effort::High => Some("high".to_owned()),
        }
    }
}

/// Which model answers, kept with the person's preferences. There is no
/// credential here and none is asked for: the provider's own sign-in is used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: Provider,
    pub effort: Effort,
    /// Empty leaves the choice to the provider.
    pub claude_model: String,
    pub codex_model: String,
    /// Where the provider's program is, when it is not on the search path.
    pub claude_executable: Option<PathBuf>,
    pub codex_executable: Option<PathBuf>,
    /// Requests come back as proposals to review, made by a planner that
    /// sees only the project's data. Otherwise the agent edits as it works,
    /// with every tool an outside agent has.
    pub review: bool,
    /// Folders the person lets the agent use besides the project's own and
    /// their usual places, as `--root` does for an outside agent.
    pub folders: Vec<PathBuf>,
}

impl AiSettings {
    /// The model named for the provider in use.
    pub fn model(&self) -> &str {
        match self.provider {
            Provider::Claude => &self.claude_model,
            Provider::Codex => &self.codex_model,
        }
    }

    pub fn set_model(&mut self, model: &str) {
        let model = clean_model(model);
        match self.provider {
            Provider::Claude => self.claude_model = model,
            Provider::Codex => self.codex_model = model,
        }
    }

    pub fn sanitized(mut self) -> Self {
        self.claude_model = clean_model(&self.claude_model);
        self.codex_model = clean_model(&self.codex_model);
        self
    }

    /// One line for the panel: who answers and how.
    pub fn describe(&self) -> String {
        let mut text = self.provider.name().to_owned();
        if !self.model().is_empty() {
            text.push_str(" · ");
            text.push_str(self.model());
        }
        if self.effort != Effort::Default {
            text.push_str(" · ");
            text.push_str(self.effort.name());
        }
        text
    }

    fn config(&self) -> AiConfig {
        let model = self.model();
        AiConfig {
            provider: self.provider.runtime(),
            executable: match self.provider {
                Provider::Claude => self.claude_executable.clone(),
                Provider::Codex => self.codex_executable.clone(),
            },
            model: (!model.is_empty()).then(|| model.to_owned()),
            effort: self.effort.value(),
            ..AiConfig::default()
        }
    }
}

/// Request ids of this window's own agent begin with this, so its edits
/// are told apart from those of an agent that connected from outside.
pub const TAG: &str = "neptune-video-tab";

/// The tag this window's agent begins its request ids with: the window's
/// own, since another window's agent may also edit the project shown here.
pub fn window_tag(instance: Uuid) -> String {
    format!("{TAG}-{}", instance.simple())
}

/// Whether a request came from the agent of the window tagged `tag`.
pub fn own_request(tag: &str, request_id: &str) -> bool {
    request_id
        .strip_prefix(tag)
        .is_some_and(|rest| rest.starts_with('-'))
}

/// A request the agent is answering.
pub struct Turn {
    id: Uuid,
    pub provider: Provider,
    pub since: Instant,
    /// The provider's process has the request.
    pub started: bool,
    pub cancelling: bool,
    /// Edits it made so far, as this window counted them.
    pub edits: usize,
    /// Tool calls that said they changed a project, this window's or
    /// another one the agent made or opened.
    pub changed: usize,
}

/// What the agent did, as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Heard {
    /// The request reached the provider; `restarted` when it began a new
    /// conversation although nobody ended the last one, because the
    /// provider or its settings changed.
    Started {
        restarted: bool,
    },
    Said(String),
    /// A tool call began, in the words the panel uses.
    Called {
        call: String,
        what: String,
    },
    /// A tool call ended: its edit's label, or why it was refused.
    Returned {
        call: String,
        ok: bool,
        detail: Option<String>,
    },
    /// Answered, with who answered and how many edits it made in this
    /// window and in other projects.
    Finished {
        by: String,
        edits: usize,
        elsewhere: usize,
    },
    Failed(String),
    Cancelled {
        edits: usize,
        elsewhere: usize,
    },
}

/// The agent that edits this window's project as it works.
pub struct Agent {
    conversation: Result<Conversation, String>,
    turn: Option<Turn>,
    /// A conversation has been started and not ended.
    talking: bool,
    roots: Option<RootsFile>,
}

/// The file naming the folders the agent may use, which its server reads
/// again whenever it changes: written whole on a thread of its own, so the
/// window never waits on the disk while it draws.
struct RootsFile {
    path: PathBuf,
    sent: Option<Vec<PathBuf>>,
    writer: mpsc::Sender<(Vec<PathBuf>, Option<mpsc::SyncSender<()>>)>,
}

impl RootsFile {
    fn new(path: PathBuf) -> Option<Self> {
        let (writer, asked) = mpsc::channel::<(Vec<PathBuf>, Option<mpsc::SyncSender<()>>)>();
        let file = path.clone();
        std::thread::Builder::new()
            .name("neptune-video-agent-roots".into())
            .spawn(move || {
                while let Ok((roots, done)) = asked.recv() {
                    let _ = write_roots(&file, &roots);
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
                let _ = std::fs::remove_file(&file);
            })
            .ok()?;
        Some(Self {
            path,
            sent: None,
            writer,
        })
    }
}

/// Replaces `file` with `{"roots": [...]}`, readable by its owner only.
fn write_roots(file: &Path, roots: &[PathBuf]) -> std::io::Result<()> {
    let named: Vec<&str> = roots.iter().filter_map(|root| root.to_str()).collect();
    let bytes = serde_json::to_vec(&serde_json::json!({ "roots": named }))?;
    let part = file.with_extension(format!("roots-part-{}", Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&part).and_then(|mut out| {
        use std::io::Write;
        out.write_all(&bytes)?;
        out.sync_all()
    });
    match written.and_then(|()| std::fs::rename(&part, file)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&part);
            Err(error)
        }
    }
}

impl Agent {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            conversation: Conversation::new(wake).map_err(|error| error.to_string()),
            turn: None,
            talking: false,
            roots: None,
        }
    }

    /// Names in `file` the folders the agent may use, when they changed.
    /// `wait` waits, a second at most, until the file is written: a server
    /// about to start reads it at once. Only a change is ever waited for.
    pub fn set_roots(&mut self, file: &Path, wanted: Vec<PathBuf>, wait: bool) {
        if self.roots.as_ref().is_none_or(|roots| roots.path != file) {
            self.roots = RootsFile::new(file.to_path_buf());
        }
        let Some(roots) = &mut self.roots else {
            return;
        };
        if roots.sent.as_ref() == Some(&wanted) {
            return;
        }
        let (done, written) = if wait {
            let (done, written) = mpsc::sync_channel(1);
            (Some(done), Some(written))
        } else {
            (None, None)
        };
        if roots.writer.send((wanted.clone(), done)).is_ok() {
            roots.sent = Some(wanted);
            if let Some(written) = written {
                let _ = written.recv_timeout(Duration::from_secs(1));
            }
        }
    }

    /// Whether the window keeps a roots file for its agent now.
    pub fn follows_roots(&self) -> bool {
        self.roots.is_some() && (self.talking || self.turn.is_some())
    }

    pub fn turn(&self) -> Option<&Turn> {
        self.turn.as_ref()
    }

    /// Whether a conversation goes on, so the next request continues it.
    pub fn talking(&self) -> bool {
        self.talking
    }

    /// Sends `prompt`, continuing the conversation when there is one with
    /// the same provider. `server` is the tool server started in this window.
    pub fn ask(
        &mut self,
        settings: &AiSettings,
        server: ToolServer,
        prompt: String,
    ) -> Result<(), String> {
        if self.turn.is_some() {
            return Err("The agent is still working. Stop it first.".to_owned());
        }
        let conversation = self
            .conversation
            .as_ref()
            .map_err(|problem| format!("The agent cannot be started: {problem}"))?;
        let id = conversation
            .ask(settings.config(), server, prompt)
            .map_err(|error| error.to_string())?;
        self.turn = Some(Turn {
            id,
            provider: settings.provider,
            since: Instant::now(),
            started: false,
            cancelling: false,
            edits: 0,
            changed: 0,
        });
        Ok(())
    }

    /// Stops the request. Its provider stops at once and the conversation
    /// ends; edits already made stay, each a step of the history.
    pub fn stop(&mut self) {
        if let (Ok(conversation), Some(turn)) = (&self.conversation, &mut self.turn)
            && !turn.cancelling
        {
            turn.cancelling = true;
            conversation.stop();
        }
    }

    /// Ends the conversation: the next request starts a new one.
    pub fn end(&mut self) {
        if let Ok(conversation) = &self.conversation {
            conversation.end();
        }
        if let Some(turn) = &mut self.turn {
            turn.cancelling = true;
        }
        self.talking = false;
    }

    /// An edit of this agent's landed in the window.
    pub fn count_edit(&mut self) {
        if let Some(turn) = &mut self.turn {
            turn.edits += 1;
        }
    }

    /// The next thing the agent did in the request being answered.
    pub fn poll(&mut self) -> Option<Heard> {
        loop {
            let event = self.conversation.as_ref().ok()?.poll()?;
            let Some(turn) = self.turn.as_mut().filter(|turn| turn.id == event.turn()) else {
                continue;
            };
            let edits = turn.edits;
            let elsewhere = turn.changed.saturating_sub(turn.edits);
            let provider = turn.provider;
            return Some(match event {
                TurnEvent::Started { fresh, .. } => {
                    turn.started = true;
                    let restarted = fresh && self.talking;
                    self.talking = true;
                    Heard::Started { restarted }
                }
                TurnEvent::Said { text, .. } => Heard::Said(text),
                TurnEvent::Called { call, tool, op, .. } => Heard::Called {
                    call,
                    what: step_words(&tool, op.as_deref()),
                },
                TurnEvent::Returned {
                    call, ok, detail, ..
                } => {
                    // A result that names its edit is a change, wherever
                    // it was made.
                    if ok && detail.is_some() {
                        turn.changed += 1;
                    }
                    Heard::Returned { call, ok, detail }
                }
                TurnEvent::Finished { model, .. } => {
                    self.turn = None;
                    Heard::Finished {
                        by: match model.filter(|model| !model.is_empty()) {
                            Some(model) => format!("{} · {model}", provider.name()),
                            None => provider.name().to_owned(),
                        },
                        edits,
                        elsewhere,
                    }
                }
                TurnEvent::Failed { message, .. } => {
                    self.turn = None;
                    self.talking = false;
                    Heard::Failed(message)
                }
                TurnEvent::Cancelled { .. } => {
                    self.turn = None;
                    self.talking = false;
                    Heard::Cancelled { edits, elsewhere }
                }
            });
        }
    }
}

/// `neptune-video-mcp`: named by `NEPTUNE_VIDEO_MCP`, else the one installed beside this
/// program (links followed, so a linked `neptune-video` finds its own).
pub fn tool_program() -> Result<PathBuf, String> {
    if let Some(named) = std::env::var_os("NEPTUNE_VIDEO_MCP").filter(|named| !named.is_empty()) {
        let named = PathBuf::from(named);
        return if named.is_file() {
            Ok(named)
        } else {
            Err(format!(
                "NEPTUNE_VIDEO_MCP names {}, which is not a program",
                named.display()
            ))
        };
    }
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|error| format!("Cannot find where Neptune Video is installed: {error}"))?;
    let beside = exe.with_file_name(format!("neptune-video-mcp{}", std::env::consts::EXE_SUFFIX));
    if beside.is_file() {
        Ok(beside)
    } else {
        Err(format!(
            "The agent's editing tools, {}, are missing. Reinstall Neptune Video, or build them with \
             cargo build --bins.",
            beside.display()
        ))
    }
}

/// A tool call in the words of the panel.
pub fn step_words(tool: &str, op: Option<&str>) -> String {
    let words = match (tool, op) {
        ("project_status" | "project_inspect", _) => "Looked at the project",
        ("project_settings", _) => "Project settings",
        ("project_save", _) => "Saved the project",
        ("project_create" | "project_open" | "project_close" | "desktop", _) => "Projects",
        ("media_import", _) => "Imported media",
        ("media_inspect", _) => "Inspected media",
        ("media_remove", _) => "Removed media",
        ("media_folder", _) => "Library folders",
        ("track_edit", _) => "Tracks",
        ("clip_add", _) => "Added clips",
        ("clip_edit", Some("trim")) => "Trimmed clips",
        ("clip_edit", Some("split")) => "Split clips",
        ("clip_edit", Some("move")) => "Moved clips",
        ("clip_edit", Some("delete")) => "Deleted clips",
        ("clip_edit", _) => "Edited clips",
        ("marker_edit", _) => "Markers",
        ("effects_catalog", _) => "Looked at effects",
        ("effect_edit", _) => "Effects",
        ("transition_set", _) => "Transition",
        ("retime_set", _) => "Speed",
        ("keyframes_set", _) => "Animation",
        ("text_add", _) => "Added text",
        ("text_edit", _) => "Text style",
        ("captions", _) => "Captions",
        ("templates", _) => "Templates",
        ("audio_edit", _) => "Sound",
        ("transcription_models", _) => "Speech models",
        ("transcribe_start", _) => "Transcribing speech",
        ("frame_render", _) => "Looked at frames",
        ("contact_sheet", _) => "Looked at a contact sheet",
        ("scenes_start", _) => "Finding scene cuts",
        ("loudness_start", _) => "Measuring loudness",
        ("render_capabilities", _) => "Export options",
        ("render_export_start", _) => "Exporting",
        ("job", Some("wait")) => "Waiting for a job",
        ("job", _) => "Jobs",
        ("history", Some("undo")) => "Undid an edit",
        ("history", Some("redo")) => "Redid an edit",
        ("history", _) => "History",
        ("checkpoint", _) => "Checkpoint",
        ("edit_apply", _) => "Edited",
        ("edit_schema", _) => "Read the editing reference",
        (other, _) => return other.replace('_', " "),
    };
    words.to_owned()
}

/// A model name as it is handed to a command line: one short printable word.
fn clean_model(model: &str) -> String {
    model
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .take(MODEL_LIMIT)
        .collect()
}

/// A request that has been sent and not answered.
pub struct Waiting {
    id: Uuid,
    pub provider: Provider,
    pub since: Instant,
    /// The runtime reported that the provider's process is running.
    pub started: bool,
    /// Cancellation was asked for and has not been confirmed.
    pub cancelling: bool,
}

/// A proposal as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub id: Uuid,
    pub summary: String,
    /// What each command does, in the words the history uses.
    pub steps: Vec<String>,
    /// What applying it would change, in counts and lengths.
    pub changes: Vec<String>,
    /// The provider and model that planned it.
    pub by: String,
}

/// What applying a proposal did to the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The history took one step, which undo takes back.
    Changed,
    /// The edits were valid and left the document as it was: no step was
    /// added, so there is nothing to undo and redo is kept.
    Unchanged,
}

/// What the runtime reported about the request being waited for.
pub enum Update {
    Planned(Plan),
    Failed(String),
    Cancelled,
}

pub struct Planner {
    runtime: Result<AiRuntime, String>,
    waiting: Option<Waiting>,
    /// The one proposal that can still be applied, under its request's id.
    open: Option<(Uuid, Box<AiProposal>)>,
}

impl Planner {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            runtime: AiRuntime::new(wake).map_err(|error| error.to_string()),
            waiting: None,
            open: None,
        }
    }

    pub fn waiting(&self) -> Option<&Waiting> {
        self.waiting.as_ref()
    }

    /// Sends `prompt` with a snapshot of the document as it stands. The model
    /// is given the project's editing data, never the bytes of its media.
    pub fn ask(
        &mut self,
        settings: &AiSettings,
        editor: &Editor,
        selected: &[ClipId],
        playhead: Time,
        prompt: String,
    ) -> Result<(), String> {
        if self.waiting.is_some() {
            return Err("A request is still being planned. Cancel it first.".to_owned());
        }
        let runtime = self
            .runtime
            .as_ref()
            .map_err(|problem| format!("The agent cannot be started: {problem}"))?;
        let context = AiContext::capture(editor, selected, playhead);
        let id = runtime
            .request(settings.config(), context, prompt)
            .map_err(|error| error.to_string())?;
        self.waiting = Some(Waiting {
            id,
            provider: settings.provider,
            since: Instant::now(),
            started: false,
            cancelling: false,
        });
        Ok(())
    }

    /// Asks the runtime to stop. The request is over when it says so.
    pub fn cancel(&mut self) {
        if let (Ok(runtime), Some(waiting)) = (&self.runtime, &mut self.waiting)
            && !waiting.cancelling
        {
            waiting.cancelling = true;
            runtime.cancel();
        }
    }

    /// The next thing the runtime reported about the request being waited
    /// for. Reports about any other request are dropped.
    pub fn poll(&mut self) -> Option<Update> {
        loop {
            let event = self.runtime.as_ref().ok()?.poll()?;
            let waited = self.waiting.as_ref().map(|waiting| waiting.id);
            match event {
                AiEvent::Started { id } => {
                    if let Some(waiting) = self.waiting.as_mut().filter(|w| w.id == id) {
                        waiting.started = true;
                    }
                }
                AiEvent::Proposal { id, proposal } if waited == Some(id) => {
                    self.waiting = None;
                    let plan = plan_of(id, &proposal);
                    if !plan.steps.is_empty() {
                        self.open = Some((id, proposal));
                    }
                    return Some(Update::Planned(plan));
                }
                AiEvent::Failed { id, message } if waited == Some(id) => {
                    self.waiting = None;
                    return Some(Update::Failed(message));
                }
                AiEvent::Cancelled { id } if waited == Some(id) => {
                    self.waiting = None;
                    return Some(Update::Cancelled);
                }
                _ => {}
            }
        }
    }

    pub fn open(&self) -> Option<Uuid> {
        self.open.as_ref().map(|(id, _)| *id)
    }

    /// Whether the open proposal is still about the document as it stands.
    pub fn open_is_current(&self, editor: &Editor) -> bool {
        self.open
            .as_ref()
            .is_some_and(|(_, proposal)| proposal.is_current(editor))
    }

    /// Applies proposal `id` as one step of the history. The proposal checks
    /// the project, editing context and revision it was planned for; a
    /// refusal leaves the document as it was. Either way it is closed. What
    /// comes back is what the editor reported, not what the summary promised.
    pub fn apply(&mut self, id: Uuid, editor: &mut Editor) -> Result<Applied, String> {
        let (_, proposal) = self
            .open
            .take_if(|(open, _)| *open == id)
            .ok_or("That proposal is no longer open.")?;
        if !proposal.is_current(editor) {
            return Err("The project changed since this was planned. Ask again.".to_owned());
        }
        proposal
            .apply(editor)
            .map(|outcome| {
                if outcome.changed {
                    Applied::Changed
                } else {
                    Applied::Unchanged
                }
            })
            .map_err(|error| error.to_string())
    }

    /// Closes proposal `id` without applying it.
    pub fn discard(&mut self, id: Uuid) -> bool {
        self.open.take_if(|(open, _)| *open == id).is_some()
    }

    /// The document was replaced: nothing asked or proposed is about the new
    /// one. Returns the proposal that was open.
    pub fn forget(&mut self) -> Option<Uuid> {
        self.cancel();
        self.open.take().map(|(id, _)| id)
    }
}

fn plan_of(id: Uuid, proposal: &AiProposal) -> Plan {
    let provider = Provider::of(&proposal.provider).name();
    Plan {
        id,
        summary: proposal.summary.trim().to_owned(),
        steps: steps(&proposal.commands),
        changes: differences(&proposal.before, &proposal.after),
        by: match proposal.model.as_deref().filter(|model| !model.is_empty()) {
            Some(model) => format!("{provider} · {model}"),
            None => provider.to_owned(),
        },
    }
}

/// One line per command, with the commands of a batch in its place.
pub fn steps(commands: &[Command]) -> Vec<String> {
    fn walk(commands: &[Command], out: &mut Vec<String>) {
        for command in commands {
            match command {
                Command::Batch { commands, .. } => walk(commands, out),
                other => out.push(other.label().to_owned()),
            }
        }
    }
    let mut out = Vec::new();
    walk(commands, &mut out);
    out
}

/// What differs between two documents, in the counts and lengths a person
/// checks before agreeing to a change.
pub fn differences(before: &Project, after: &Project) -> Vec<String> {
    let clips = |project: &Project| project.tracks.iter().map(|t| t.clips.len()).sum::<usize>();
    let mut out = Vec::new();
    let mut count = |name: &str, was: usize, is: usize| {
        if was != is {
            out.push(format!("{name} {was} → {is}"));
        }
    };
    count("Clips", clips(before), clips(after));
    count("Tracks", before.tracks.len(), after.tracks.len());
    count("Markers", before.markers.len(), after.markers.len());
    count("Media", before.assets.len(), after.assets.len());
    count("Trackers", before.trackers.len(), after.trackers.len());
    if before.duration() != after.duration() {
        out.push(format!(
            "Length {} → {}",
            crate::timefmt::short(before.duration()),
            crate::timefmt::short(after.duration())
        ));
    }
    if before.name != after.name {
        out.push(format!("Name → {}", after.name));
    }
    if before.settings != after.settings {
        out.push("Canvas".to_owned());
    }
    if before.trackers.len() == after.trackers.len() && before.trackers != after.trackers {
        out.push("Tracker paths changed".to_owned());
    }
    looks(before, after, &mut out);
    // Something moved or changed in place: say so rather than nothing.
    if out.is_empty() && before != after {
        out.push("Clips edited in place".to_owned());
    }
    out
}

/// At most this many clips are named; the rest are counted.
const LOOK_LINES: usize = 6;

/// One line per surviving clip whose look, motion or speed map differs.
fn looks(before: &Project, after: &Project, out: &mut Vec<String>) {
    let was: HashMap<_, _> = before
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .map(|clip| (clip.id, clip))
        .collect();
    let mut more = 0;
    let mut named = 0;
    for clip in after.tracks.iter().flat_map(|track| &track.clips) {
        let Some(parts) = was.get(&clip.id).map(|was| look_changes(was, clip)) else {
            continue;
        };
        if parts.is_empty() {
            continue;
        }
        if named == LOOK_LINES {
            more += 1;
            continue;
        }
        named += 1;
        let name: String = clip.name.chars().take(32).collect();
        let name = if name.len() < clip.name.len() {
            format!("{name}…")
        } else if name.trim().is_empty() {
            "Clip".to_owned()
        } else {
            name
        };
        out.push(format!("{name}: {}", parts.join(", ")));
    }
    if more > 0 {
        out.push(format!("Look or motion of {more} more clips"));
    }
}

fn look_changes(was: &Clip, is: &Clip) -> Vec<String> {
    fn counted<T: PartialEq>(name: &str, was: &[T], is: &[T], out: &mut Vec<String>) {
        if was.len() != is.len() {
            out.push(format!("{name} {} → {}", was.len(), is.len()));
        } else if was != is {
            out.push(format!("{name} changed"));
        }
    }
    fn optional<T: PartialEq>(name: &str, was: &Option<T>, is: &Option<T>, out: &mut Vec<String>) {
        let change = match (was, is) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (was, is) if was != is => "changed",
            _ => return,
        };
        out.push(format!("{name} {change}"));
    }
    let mut out = Vec::new();
    counted("effects", &was.effects, &is.effects, &mut out);
    counted("masks", &was.masks, &is.masks, &mut out);
    if was.blend != is.blend {
        let mode = serde_json::to_value(is.blend).unwrap_or_default();
        out.push(format!(
            "blend {}",
            mode.as_str().unwrap_or("changed").replace('_', " ")
        ));
    }
    let keys = |clip: &Clip| clip.animation.iter().map(|track| track.keys.len()).sum();
    let (keys_was, keys_is): (usize, usize) = (keys(was), keys(is));
    if keys_was != keys_is {
        out.push(format!("keyframes {keys_was} → {keys_is}"));
    } else if was.animation != is.animation {
        out.push("keyframes changed".to_owned());
    }
    optional("speed map", &was.retime, &is.retime, &mut out);
    optional(
        "transition in",
        &was.transition_in,
        &is.transition_in,
        &mut out,
    );
    optional(
        "transition out",
        &was.transition_out,
        &is.transition_out,
        &mut out,
    );
    optional("tracking", &was.follow, &is.follow, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Clip, Track, TrackKind, Transform};

    #[test]
    fn a_window_knows_its_own_agents_requests_from_another_windows() {
        let (here, there) = (Uuid::new_v4(), Uuid::new_v4());
        let tag = window_tag(here);
        assert!(own_request(&tag, &format!("{tag}-3")));
        assert!(!own_request(&tag, &format!("{}-3", window_tag(there))));
        assert!(!own_request(&tag, "neptune-video-tab-3"));
        assert!(!own_request(&tag, "mcp-3"));
        assert!(!own_request(&tag, &format!("{tag}3")));
    }

    #[test]
    fn the_agents_folders_are_written_whole_for_its_owner_only_and_replaced_when_they_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("window.roots");
        let mut agent = Agent::new(|| {});
        let read = || -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap()
        };
        agent.set_roots(&file, vec![PathBuf::from("/a")], true);
        assert_eq!(read(), serde_json::json!({"roots": ["/a"]}));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        agent.set_roots(&file, vec![PathBuf::from("/a"), PathBuf::from("/b")], true);
        assert_eq!(read(), serde_json::json!({"roots": ["/a", "/b"]}));
        // Nothing but the file is left beside it.
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1);
        // The window that closes takes its file with it.
        drop(agent);
        for _ in 0..100 {
            if !file.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!file.exists());
    }

    #[test]
    fn a_model_name_is_one_printable_word() {
        assert_eq!(clean_model("  claude-opus-5-5\n"), "claude-opus-5-5");
        assert_eq!(clean_model("a b\tc\u{7}"), "abc");
        assert_eq!(clean_model(&"x".repeat(500)).len(), MODEL_LIMIT);
    }

    #[test]
    fn each_provider_keeps_its_own_model() {
        let mut settings = AiSettings::default();
        settings.set_model("claude-opus-5-5");
        settings.provider = Provider::Codex;
        assert_eq!(settings.model(), "");
        settings.set_model("gpt-5");
        settings.provider = Provider::Claude;
        assert_eq!(settings.model(), "claude-opus-5-5");
        assert_eq!(settings.describe(), "Claude · claude-opus-5-5 · High");
        settings.provider = Provider::Codex;
        settings.effort = Effort::Default;
        assert_eq!(settings.describe(), "Codex · gpt-5");
    }

    #[test]
    fn the_request_carries_the_choices_and_no_more() {
        let mut settings = AiSettings::default();
        let config = settings.config();
        assert!(matches!(config.provider, AiProvider::Claude));
        assert_eq!(config.model, None);
        assert_eq!(config.effort.as_deref(), Some("high"));
        assert_eq!(config.executable, None);
        // The deadline stays the runtime's own.
        assert_eq!(config.timeout_seconds, AiConfig::default().timeout_seconds);

        settings.provider = Provider::Codex;
        settings.effort = Effort::Default;
        settings.set_model("gpt-5");
        settings.codex_executable = Some("/opt/codex".into());
        let config = settings.config();
        assert!(matches!(config.provider, AiProvider::Codex));
        assert_eq!(config.model.as_deref(), Some("gpt-5"));
        assert_eq!(config.effort, None);
        assert_eq!(config.executable, Some(PathBuf::from("/opt/codex")));
    }

    #[test]
    fn settings_written_before_the_agent_existed_still_load() {
        let settings: AiSettings = serde_json::from_str(r#"{"provider":"codex"}"#).unwrap();
        assert_eq!(settings.provider, Provider::Codex);
        assert_eq!(settings.effort, Effort::High);
        // Nothing that could hold a credential is written.
        let written = serde_json::to_string(&AiSettings::default()).unwrap();
        for word in ["key", "token", "secret", "password", "credential"] {
            assert!(!written.contains(word), "{written}");
        }
    }

    #[test]
    fn the_steps_of_a_batch_are_listed_in_its_place() {
        let track = Track::new("Text 1", TrackKind::Text);
        let commands = vec![
            Command::RenameProject {
                name: "Film".into(),
            },
            Command::Batch {
                label: "Titles".into(),
                commands: vec![
                    Command::AddClip {
                        track_id: track.id,
                        clip: Clip::text("Hello", Time::ZERO, Time::from_seconds(2.0)),
                    },
                    Command::AddTrack { track },
                ],
            },
        ];
        assert_eq!(
            steps(&commands),
            ["Rename project", "Add clip", "Add track"]
        );
        assert!(steps(&[]).is_empty());
    }

    #[test]
    fn the_differences_say_what_a_proposal_would_change() {
        let before = Project::default();
        assert!(differences(&before, &before).is_empty());

        let mut editor = Editor::new(before.clone()).unwrap();
        let track = Track::new("Titles", TrackKind::Text);
        let clip = Clip::text("Hello", Time::ZERO, Time::from_seconds(7.0));
        let clip_id = clip.id;
        editor
            .dispatch(Command::Batch {
                label: "Title".into(),
                commands: vec![
                    Command::AddTrack {
                        track: track.clone(),
                    },
                    Command::AddClip {
                        track_id: track.id,
                        clip,
                    },
                    Command::RenameProject {
                        name: "Film".into(),
                    },
                ],
            })
            .unwrap();
        let after = editor.project().clone();
        let tracks = before.tracks.len();
        assert_eq!(
            differences(&before, &after),
            [
                "Clips 0 → 1".to_owned(),
                format!("Tracks {tracks} → {}", tracks + 1),
                "Length 0:00 → 0:07".to_owned(),
                "Name → Film".to_owned(),
            ]
        );

        // A change that moves no count is still named.
        editor
            .dispatch(Command::SetTransform {
                clip_id,
                transform: Transform {
                    opacity: 0.5,
                    ..Transform::default()
                },
            })
            .unwrap();
        assert_eq!(
            differences(&after, editor.project()),
            ["Clips edited in place"]
        );
    }

    #[test]
    fn look_and_motion_changes_are_named_per_clip() {
        use editor_core::{
            Asset, BlendMode, Ease, Effect, EffectKind, Keyframe, MediaKind, ParamKey, Retime,
            Transition, TransitionEdge, TransitionKind,
        };
        let mut project = Project::default();
        let asset = Asset::new("/media/shot.mp4".into(), MediaKind::Video, Time(8_000_000));
        let mut ids = Vec::new();
        for index in 0..LOOK_LINES as i64 + 2 {
            let mut clip = Clip::from_asset(&asset, Time(index * 1_000_000));
            clip.duration = Time(1_000_000);
            clip.name = format!("Shot {index}");
            ids.push(clip.id);
            std::sync::Arc::make_mut(&mut project.tracks[0])
                .clips
                .push(clip);
        }
        std::sync::Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        let mut editor = Editor::new(project).unwrap();
        let before = editor.project().clone();
        let clip_id = ids[0];
        let key = |at, value| Keyframe {
            at: Time(at),
            value,
            ease: Ease::Linear,
        };
        editor
            .dispatch(Command::Batch {
                label: "Look".into(),
                commands: vec![
                    Command::SetEffects {
                        clip_id,
                        effects: vec![
                            Effect::new(EffectKind::Blur),
                            Effect::new(EffectKind::Grain),
                        ],
                    },
                    Command::SetBlendMode {
                        clip_id,
                        mode: BlendMode::SoftLight,
                    },
                    Command::SetKeyframes {
                        clip_id,
                        target: None,
                        key: ParamKey::Opacity,
                        keys: vec![key(0, 0.0), key(500_000, 1.0)],
                    },
                    Command::SetTransition {
                        clip_id,
                        edge: TransitionEdge::In,
                        transition: Some(Transition {
                            kind: TransitionKind::Dissolve,
                            duration: Time(250_000),
                            ease: Ease::Linear,
                            params: vec![],
                        }),
                    },
                    Command::SetRetime {
                        clip_id: ids[1],
                        retime: Some(Retime::constant(Time(1_000_000), 1.0)),
                        duration: Time(1_000_000),
                    },
                ],
            })
            .unwrap();
        let after = editor.project().clone();
        assert_eq!(
            differences(&before, &after),
            [
                "Shot 0: effects 0 → 2, blend soft light, keyframes 0 → 2, transition in added",
                "Shot 1: speed map added",
            ]
        );
        assert_eq!(
            differences(&after, &before),
            [
                "Shot 0: effects 2 → 0, blend normal, keyframes 2 → 0, transition in removed",
                "Shot 1: speed map removed",
            ]
        );

        // Same counts, different values.
        editor
            .dispatch(Command::SetKeyframes {
                clip_id,
                target: None,
                key: ParamKey::Opacity,
                keys: vec![key(0, 0.5), key(500_000, 1.0)],
            })
            .unwrap();
        assert_eq!(
            differences(&after, editor.project()),
            ["Shot 0: keyframes changed"]
        );

        // Past the named clips the rest are counted.
        let mut all = before.clone();
        for clip in &mut std::sync::Arc::make_mut(&mut all.tracks[0]).clips {
            clip.blend = BlendMode::Screen;
        }
        let lines = differences(&before, &all);
        assert_eq!(lines.len(), LOOK_LINES + 1);
        assert_eq!(lines[0], "Shot 0: blend screen");
        assert_eq!(lines[LOOK_LINES], "Look or motion of 2 more clips");
    }

    /// A planner whose provider is an owned script that prints one answer:
    /// the proposal is the runtime's own, validated as any other.
    #[cfg(unix)]
    fn planned(editor: &Editor, commands: &str) -> (Planner, Plan) {
        use std::{os::unix::fs::PermissionsExt, time::Duration};

        let dir = std::env::temp_dir().join(format!("neptune-video-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join("provider");
        let answer = format!(
            r#"{{"type":"result","subtype":"success","structured_output":{{"summary":"Planned","commands":{commands}}}}}"#
        );
        std::fs::write(
            &executable,
            format!("#!/bin/sh\ncat >/dev/null; printf '%s\\n' '{answer}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();

        let settings = AiSettings {
            claude_executable: Some(executable),
            ..AiSettings::default()
        };
        let mut planner = Planner::new(|| {});
        planner
            .ask(&settings, editor, &[], Time::ZERO, "Plan".to_owned())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let update = loop {
            if let Some(update) = planner.poll() {
                break update;
            }
            assert!(Instant::now() < deadline, "the provider did not answer");
            std::thread::sleep(Duration::from_millis(10));
        };
        let _ = std::fs::remove_dir_all(&dir);
        match update {
            Update::Planned(plan) => (planner, plan),
            Update::Failed(message) => panic!("failed: {message}"),
            Update::Cancelled => panic!("cancelled"),
        }
    }

    /// A document at its saved state with one step waiting to be redone.
    #[cfg(unix)]
    fn with_redo() -> Editor {
        let mut editor = Editor::new(Project::new("Original")).unwrap();
        editor
            .dispatch(Command::RenameProject {
                name: "Redo me".into(),
            })
            .unwrap();
        assert!(editor.undo());
        editor
    }

    #[cfg(unix)]
    #[test]
    fn a_proposal_that_changes_nothing_is_reported_as_unchanged() {
        let mut editor = with_redo();
        let (root, revision) = (editor.snapshot(), editor.revision());
        let (mut planner, plan) = planned(
            &editor,
            r#"[{"type":"rename_project","name":"Temporary"},{"type":"rename_project","name":"Original"}]"#,
        );
        // It has steps, so it is offered; nothing says it would change anything.
        assert_eq!(plan.steps, ["Rename project", "Rename project"]);
        assert!(plan.changes.is_empty());
        assert_eq!(planner.open(), Some(plan.id));
        assert!(planner.open_is_current(&editor));

        assert_eq!(planner.apply(plan.id, &mut editor), Ok(Applied::Unchanged));
        assert_eq!(editor.revision(), revision);
        assert!(std::sync::Arc::ptr_eq(&root, &editor.snapshot()));
        assert!(!editor.is_dirty());
        assert!(!editor.can_undo());
        assert!(editor.can_redo());
        // It is closed like any applied proposal.
        assert_eq!(planner.open(), None);
        assert!(planner.apply(plan.id, &mut editor).is_err());
        assert!(editor.redo());
        assert_eq!(editor.project().name, "Redo me");
    }

    #[cfg(unix)]
    #[test]
    fn a_proposal_that_edits_is_one_step_undo_takes_back() {
        let mut editor = with_redo();
        let (root, revision) = (editor.snapshot(), editor.revision());
        let (mut planner, plan) = planned(
            &editor,
            r#"[{"type":"rename_project","name":"Draft"},{"type":"rename_project","name":"Reviewed"}]"#,
        );
        assert_eq!(plan.changes, ["Name → Reviewed"]);

        assert_eq!(planner.apply(plan.id, &mut editor), Ok(Applied::Changed));
        assert_eq!(editor.project().name, "Reviewed");
        assert_eq!(editor.revision(), revision + 1);
        assert!(editor.is_dirty());
        assert!(!editor.can_redo());
        assert_eq!(planner.open(), None);

        assert!(editor.undo());
        assert!(std::sync::Arc::ptr_eq(&root, &editor.snapshot()));
        assert!(!editor.can_undo());
        assert!(!editor.is_dirty());
        assert!(editor.redo());
        assert_eq!(editor.project().name, "Reviewed");
    }

    #[cfg(unix)]
    #[test]
    fn a_proposal_about_an_earlier_document_is_refused_not_unchanged() {
        let mut editor = with_redo();
        let (mut planner, plan) = planned(
            &editor,
            r#"[{"type":"rename_project","name":"Temporary"},{"type":"rename_project","name":"Original"}]"#,
        );
        editor
            .dispatch(Command::RenameProject {
                name: "By hand".into(),
            })
            .unwrap();
        let (root, revision) = (editor.snapshot(), editor.revision());
        assert!(!planner.open_is_current(&editor));

        assert!(planner.apply(plan.id, &mut editor).is_err());
        assert_eq!(editor.revision(), revision);
        assert!(std::sync::Arc::ptr_eq(&root, &editor.snapshot()));
        assert_eq!(planner.open(), None);
    }
}
