//! The one project this server edits, and where it is edited: in the
//! server's own editor (Local) or in a running Aster window (Desktop). One
//! thread owns the workspace; tools reach it through [`Authority`], so
//! concurrent clients are served one at a time and a window is sent one
//! request at a time.
//!
//! Every edit is a guarded `AgentRequest` carrying the project and editing
//! context this session chose, never ones it has just read, so an edit
//! cannot land on a project that replaced the chosen one.
use super::{
    errors::{ToolError, codes},
    paths::Roots,
};
use crate::headless::{self, HeadlessDocument};
use editor_agent::{
    AgentOperation, AgentRequest, AgentResponse, AgentResult, DocumentInfo, SessionDescriptor,
};
use editor_core::{Command, Editor, PreparedEdit, Project};
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// How long a window may take to answer an edit: longer than the host's own
/// five seconds, so its `timeout` answer arrives rather than our silence.
const DESKTOP_READ: Duration = Duration::from_secs(10);
/// Looking for windows is brief: a window that does not answer this fast is
/// not counted.
const DISCOVERY_CONNECT: Duration = Duration::from_millis(500);
const DISCOVERY_READ: Duration = Duration::from_secs(3);
/// Windows are looked for again at most this often.
const DISCOVERY_EVERY: Duration = Duration::from_secs(1);
/// The most session files read in one look.
const MOST_SESSIONS: usize = 64;

/// Where the project is edited.
pub enum Backend {
    Empty,
    Local(Local),
    Desktop(Desktop),
}

pub struct Local {
    pub editor: Editor,
    pub document: Option<HeadlessDocument>,
    /// A save was refused because the file changed under the server: no
    /// edit is made until the project is opened again.
    pub conflicted: bool,
}

pub struct Desktop {
    pub descriptor: SessionDescriptor,
}

/// The project this session chose: edits carry these guards.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub project_id: Uuid,
    pub context_id: Uuid,
    pub path: Option<PathBuf>,
}

/// A window that answered.
#[derive(Debug, Clone)]
pub struct Session {
    pub file: PathBuf,
    pub descriptor: SessionDescriptor,
    pub document: DocumentInfo,
}

/// The project as one request saw it.
#[derive(Clone)]
pub struct Snapshot {
    pub project: Arc<Project>,
    pub revision: u64,
    pub context_id: Uuid,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// What an edit did.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Edited {
    pub project_id: Uuid,
    pub context_id: Uuid,
    /// The project's revision after the edit.
    pub revision: u64,
    /// `local` (this server saves the file after every edit) or `desktop`
    /// (the edit is in the open window, unsaved until saved).
    pub authority: String,
    /// The history entry's label.
    pub label: String,
    /// Whether the project changed; an edit that changes nothing adds no
    /// history entry.
    pub changed: bool,
    /// The IDs the edit added: tracks, clips, assets, folders, markers.
    pub created: Vec<Uuid>,
    /// Set for a dry run: nothing was applied.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// Things the agent should know: a handover to a window, a recovery
    /// copy that could not be written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
}

/// A history entry this server made: the editing context and the revisions
/// before and after it. Undo and redo move the revision on, so the entry
/// beneath one taken back is renumbered when the two were made one after
/// the other.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Own {
    context: Uuid,
    before: u64,
    after: u64,
}

pub struct Workspace {
    pub roots: Arc<Roots>,
    pub backend: Backend,
    binding: Option<Binding>,
    /// The entries this server applied, and those it undid, so undo and redo
    /// touch only the agent's own work.
    own_undo: Vec<Own>,
    own_redo: Vec<Own>,
    /// The folder windows publish their sessions in.
    sessions: Option<PathBuf>,
    /// Session files of windows this server launched.
    launched: Vec<PathBuf>,
    seen: Option<(Instant, Vec<Session>)>,
    notices: Vec<String>,
    requests: u64,
    /// The local render queue, made at the first export.
    pub renders: Option<crate::export::queue::RenderQueue>,
    /// The `aster` program `desktop` `open` launches, and the settings it
    /// is given.
    pub desktop_program: Option<PathBuf>,
    pub desktop_settings: Option<PathBuf>,
    pub launched_window: Option<std::process::Child>,
    /// The session file of the window this server was started for
    /// (`--window`): it works there first, and comes back there when it
    /// is told to attach and names no window.
    home: Option<PathBuf>,
    /// It still works in that window's project, so it follows the window
    /// when the window shows no project of its own yet.
    following: bool,
    /// What request ids begin with, so a window can tell whose they are.
    tag: String,
}

impl Workspace {
    pub fn new(roots: Arc<Roots>, sessions: Option<PathBuf>) -> Self {
        Self {
            roots,
            backend: Backend::Empty,
            binding: None,
            own_undo: Vec::new(),
            own_redo: Vec::new(),
            sessions,
            launched: Vec::new(),
            seen: None,
            notices: Vec::new(),
            requests: 0,
            renders: None,
            desktop_program: None,
            desktop_settings: None,
            launched_window: None,
            home: None,
            following: false,
            tag: "mcp".to_owned(),
        }
    }

    /// Request ids begin with `tag`.
    pub fn set_tag(&mut self, tag: String) {
        self.tag = tag;
    }

    /// Starts in the window whose session file is `file`: attaches now,
    /// or at the first call if the window is not answering yet. Like any
    /// server it may then make, open and close projects of its own;
    /// `desktop {op: attach}` naming no window brings it back.
    pub fn pin(&mut self, file: PathBuf) -> Result<(), ToolError> {
        self.launched.push(file.clone());
        self.home = Some(file);
        self.following = true;
        self.attach(None).map(|_| ())
    }

    /// A server still following its window, which had no project for it
    /// yet, attaches to what the window shows now.
    fn ready(&mut self) -> Result<(), ToolError> {
        if self.following && matches!(self.backend, Backend::Empty) {
            self.attach(None)?;
        }
        Ok(())
    }

    pub fn authority_name(&self) -> &'static str {
        match self.backend {
            Backend::Empty => "none",
            Backend::Local(_) => "local",
            Backend::Desktop(_) => "desktop",
        }
    }

    pub fn binding(&self) -> Option<&Binding> {
        self.binding.as_ref()
    }

    /// Notices not yet told.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    fn request(
        &mut self,
        operation: AgentOperation,
        expected_revision: Option<u64>,
    ) -> AgentRequest {
        self.requests += 1;
        AgentRequest {
            request_id: format!("{}-{}", self.tag, self.requests),
            expected_revision,
            expected_project_id: self.binding.as_ref().map(|binding| binding.project_id),
            expected_context_id: self.binding.as_ref().map(|binding| binding.context_id),
            operation,
        }
    }

    /// Sends one request to the authority. A window that does not answer
    /// an edit leaves it `indeterminate`: it may have landed.
    pub fn send(
        &mut self,
        operation: AgentOperation,
        expected_revision: Option<u64>,
    ) -> Result<AgentResponse, ToolError> {
        let mutation = operation.is_mutation() || matches!(operation, AgentOperation::Save { .. });
        let save_as = matches!(
            operation,
            AgentOperation::Save {
                destination: Some(_)
            }
        );
        self.ready()?;
        let request = self.request(operation, expected_revision);
        if let Some(response) = self.local_exports(&request) {
            return Ok(response);
        }
        match &mut self.backend {
            Backend::Empty => Err(ToolError::no_project()),
            Backend::Local(local) => {
                // Save As is how work the file no longer takes is kept.
                if mutation && local.conflicted && !save_as {
                    return Err(ToolError::new(
                        codes::PROJECT_CONFLICTED,
                        "The file changed under this server and a save was refused; nothing \
                         more is edited until project_open opens it again",
                    ));
                }
                let response = if request.operation.is_render() {
                    let editor = &local.editor;
                    let path = local
                        .document
                        .as_ref()
                        .and_then(|d| d.path().map(Path::to_path_buf));
                    let queue = self.renders.get_or_insert_with(|| {
                        crate::export::queue::RenderQueue::new(|| {}, None, None)
                    });
                    AgentResponse {
                        request_id: request.request_id.clone(),
                        revision: editor.revision(),
                        result: crate::export::wiring::render_result(
                            queue,
                            editor,
                            &request,
                            editor_core::export::builtin_presets,
                            path,
                        ),
                    }
                } else {
                    let (response, recovery) = headless::execute_keeping_result(
                        &mut local.editor,
                        &mut local.document,
                        request,
                    )?;
                    if let Some(error) = recovery {
                        self.notices.push(format!(
                            "The edit is saved, but the recovery copy that keeps its undo \
                             history could not be written: {error}"
                        ));
                    }
                    response
                };
                match &response.result {
                    AgentResult::Error { code, .. } if code == "storage_conflict" => {
                        local.conflicted = true;
                    }
                    AgentResult::Document { .. } if save_as => local.conflicted = false,
                    _ => {}
                }
                // A save that failed puts the project back as it was, in a
                // new editing context; this session keeps editing it.
                let now = local.editor.instance_id();
                if let Some(binding) = &mut self.binding
                    && binding.context_id != now
                {
                    let old = binding.context_id;
                    binding.context_id = now;
                    for own in self.own_undo.iter_mut().chain(self.own_redo.iter_mut()) {
                        if own.context == old {
                            own.context = now;
                        }
                    }
                    self.notices.push(
                        "A save failed, so the project was put back as it was before that \
                         edit; read it again before the next edit"
                            .into(),
                    );
                }
                Ok(response)
            }
            Backend::Desktop(desktop) => {
                let listing = matches!(request.operation, AgentOperation::RenderJobs);
                let size = serde_json::to_vec(&request).map_or(0, |bytes| bytes.len() as u64);
                if size + 1024 > editor_agent::MAX_REQUEST_BYTES {
                    return Err(ToolError::new(
                        codes::TOO_LARGE,
                        format!(
                            "The edit is {} MiB; a window takes at most {} MiB in one request. \
                             Split it into smaller edits.",
                            size / (1024 * 1024),
                            editor_agent::MAX_REQUEST_BYTES / (1024 * 1024)
                        ),
                    ));
                }
                let mut response = editor_agent::connect_within(
                    &desktop.descriptor,
                    request,
                    Duration::from_secs(2),
                    DESKTOP_READ,
                )
                .map_err(|error| {
                    if mutation && !matches!(error, editor_agent::AgentError::Invalid(_)) {
                        ToolError::new(
                            codes::INDETERMINATE,
                            format!(
                                "The window stopped answering after the edit was sent ({error}); \
                                 read history to learn whether it landed"
                            ),
                        )
                    } else {
                        ToolError::new(
                            codes::DESKTOP_CLOSED,
                            format!(
                                "The Aster window this project was attached to does not answer \
                                 ({error}). Open the project again with project_open or \
                                 desktop {{op: attach}}."
                            ),
                        )
                    }
                })?;
                // Exports this server started for a project it edited
                // itself keep running after the window took over.
                if listing
                    && let Some(queue) = &self.renders
                    && let AgentResult::RenderJobs { jobs } = &mut response.result
                {
                    jobs.extend(queue.jobs());
                }
                Ok(response)
            }
        }
    }

    /// The answer from this server's own render queue to a request about a
    /// job in it, when the project it was started for is no longer the one
    /// edited here.
    fn local_exports(&self, request: &AgentRequest) -> Option<AgentResponse> {
        let queue = self.renders.as_ref()?;
        let ours = match &request.operation {
            AgentOperation::RenderJobs => matches!(self.backend, Backend::Empty),
            AgentOperation::PauseRender { job }
            | AgentOperation::ResumeRender { job }
            | AgentOperation::CancelRender { job }
            | AgentOperation::RetryRender { job }
            | AgentOperation::RemoveRender { job } => {
                !matches!(self.backend, Backend::Local(_)) && queue.job(*job).is_some()
            }
            _ => false,
        };
        if !ours {
            return None;
        }
        Some(AgentResponse {
            request_id: request.request_id.clone(),
            revision: 0,
            result: crate::export::wiring::render_job_result(queue, &request.operation)?,
        })
    }

    /// The result of a request, or its refusal as a tool error. A window
    /// that now holds another project or context says `project_changed`.
    pub fn answer(&self, response: AgentResponse) -> Result<(u64, AgentResult), ToolError> {
        match response.result {
            AgentResult::Error { code, message } => {
                let desktop = matches!(self.backend, Backend::Desktop(_));
                let error = match code.as_str() {
                    "stale_project" | "stale_context" if desktop => ToolError::new(
                        codes::PROJECT_CHANGED,
                        "The window now holds another project or reloaded this one; call \
                         desktop {op: attach} to edit what it shows now",
                    ),
                    "timeout" => ToolError::new(
                        codes::BUSY,
                        "The window did not take the request in time; nothing was applied. \
                         Retry.",
                    ),
                    "indeterminate" => ToolError::new(
                        codes::INDETERMINATE,
                        "The window took the edit but did not answer in time; read history to \
                         learn whether it landed",
                    ),
                    _ => ToolError::new(&code, message),
                };
                Err(error.at(response.revision))
            }
            result => Ok((response.revision, result)),
        }
    }

    /// The project as it is now.
    pub fn snapshot(&mut self) -> Result<Snapshot, ToolError> {
        self.ready()?;
        self.check_handover()?;
        match &self.backend {
            Backend::Empty => Err(ToolError::no_project()),
            Backend::Local(local) => Ok(Snapshot {
                project: local.editor.snapshot(),
                revision: local.editor.revision(),
                context_id: local.editor.instance_id(),
                can_undo: local.editor.can_undo(),
                can_redo: local.editor.can_redo(),
            }),
            Backend::Desktop(_) => {
                let response = self.send(AgentOperation::Inspect, None)?;
                let (revision, result) = self.answer(response)?;
                let AgentResult::Project {
                    context_id,
                    project,
                    can_undo,
                    can_redo,
                    ..
                } = result
                else {
                    return Err(ToolError::unavailable("The window answered something else"));
                };
                if let Some(binding) = &self.binding
                    && (binding.project_id != project.id || binding.context_id != context_id)
                {
                    return Err(ToolError::new(
                        codes::PROJECT_CHANGED,
                        format!(
                            "The window now shows “{}”, not the project this session attached \
                             to; call desktop {{op: attach}} to edit it",
                            project.name
                        ),
                    )
                    .at(revision));
                }
                Ok(Snapshot {
                    project,
                    revision,
                    context_id,
                    can_undo,
                    can_redo,
                })
            }
        }
    }

    /// Applies one command as one history entry, after checking it against
    /// the roots and the placement rules on `snapshot`. `expected` refuses
    /// the edit unless the project is still at that revision.
    pub fn apply(
        &mut self,
        snapshot: &Snapshot,
        command: Command,
        expected: Option<u64>,
        dry_run: bool,
        allow_overlap: bool,
    ) -> Result<Edited, ToolError> {
        check_paths(&self.roots, &command, &snapshot.project)?;
        self.apply_checked(snapshot, command, expected, dry_run, allow_overlap)
    }

    /// `apply` for media `media_import` probed a moment ago inside the
    /// roots: it is not probed a second time.
    pub fn apply_imported(
        &mut self,
        snapshot: &Snapshot,
        command: Command,
        expected: Option<u64>,
        dry_run: bool,
    ) -> Result<Edited, ToolError> {
        self.apply_checked(snapshot, command, expected, dry_run, false)
    }

    fn apply_checked(
        &mut self,
        snapshot: &Snapshot,
        command: Command,
        expected: Option<u64>,
        dry_run: bool,
        allow_overlap: bool,
    ) -> Result<Edited, ToolError> {
        let prepared = PreparedEdit::from_snapshot(
            snapshot.project.clone(),
            snapshot.context_id,
            snapshot.revision,
            command.clone(),
        )
        .map_err(|error| ToolError::new("invalid_edit", error.to_string()).at(snapshot.revision))?;
        if !allow_overlap {
            let problems = new_overlaps(&prepared.before(), &prepared.after());
            if !problems.is_empty() {
                return Err(ToolError::new(
                    codes::OVERLAP,
                    "The edit would overlap clips on a track, which the window never does. \
                     Place them elsewhere, or pass allow_overlap: true.",
                )
                .with_detail(serde_json::json!({ "overlaps": problems }))
                .at(snapshot.revision));
            }
        }
        let binding = self.binding.clone().ok_or_else(ToolError::no_project)?;
        if dry_run {
            return Ok(Edited {
                project_id: binding.project_id,
                context_id: binding.context_id,
                revision: snapshot.revision,
                authority: self.authority_name().into(),
                label: prepared.label().to_owned(),
                changed: prepared.changed(),
                created: Vec::new(),
                dry_run: true,
                notices: self.take_notices(),
            });
        }
        let expected = expected.unwrap_or(snapshot.revision);
        let response = self.send(
            AgentOperation::Apply {
                command: Box::new(command),
            },
            Some(expected),
        )?;
        let (revision, result) = self.answer(response)?;
        let AgentResult::Applied {
            label,
            changed,
            created,
        } = result
        else {
            return Err(ToolError::unavailable(
                "The authority answered something else",
            ));
        };
        let context = self
            .binding
            .as_ref()
            .map_or(binding.context_id, |b| b.context_id);
        if changed {
            self.own_undo.push(Own {
                context,
                before: expected,
                after: revision,
            });
            self.own_redo.clear();
        }
        let notices = self.take_notices();
        Ok(Edited {
            project_id: binding.project_id,
            context_id: context,
            revision,
            authority: self.authority_name().into(),
            label,
            changed,
            created,
            dry_run: false,
            notices,
        })
    }

    /// Undoes or redoes the agent's own last entry, or any entry with
    /// `allow_person`.
    pub fn step(&mut self, redo: bool, allow_person: bool) -> Result<Edited, ToolError> {
        let snapshot = self.snapshot()?;
        let own = if redo {
            self.own_redo.last()
        } else {
            self.own_undo.last()
        };
        let mine = own.is_some_and(|own| {
            own.context == snapshot.context_id && own.after == snapshot.revision
        });
        if !(if redo {
            snapshot.can_redo
        } else {
            snapshot.can_undo
        }) {
            return Err(ToolError::invalid(if redo {
                "There is nothing to redo"
            } else {
                "There is nothing to undo"
            })
            .at(snapshot.revision));
        }
        if !mine && !allow_person {
            return Err(ToolError::new(
                codes::NOT_AGENT_ENTRY,
                if redo {
                    "The next redo is not an edit this agent undid; pass allow_person: true to \
                     redo it anyway"
                } else {
                    "The last edit is not one this agent made (the person, or another client, \
                     changed the project since); pass allow_person: true to undo it anyway"
                },
            )
            .at(snapshot.revision));
        }
        let operation = if redo {
            AgentOperation::Redo
        } else {
            AgentOperation::Undo
        };
        let response = self.send(operation, Some(snapshot.revision))?;
        let (revision, result) = self.answer(response)?;
        let AgentResult::History { changed } = result else {
            return Err(ToolError::unavailable(
                "The authority answered something else",
            ));
        };
        if changed {
            let (from, to) = if redo {
                (&mut self.own_redo, &mut self.own_undo)
            } else {
                (&mut self.own_undo, &mut self.own_redo)
            };
            if mine && let Some(taken) = from.pop() {
                // The entry beneath, made just before, is now the one the
                // next step takes, at the new revision.
                if let Some(beneath) = from.last_mut()
                    && beneath.context == taken.context
                    && beneath.after == taken.before
                {
                    beneath.after = revision;
                }
            }
            to.push(Own {
                context: snapshot.context_id,
                before: snapshot.revision,
                after: revision,
            });
        }
        let binding = self.binding.clone().ok_or_else(ToolError::no_project)?;
        Ok(Edited {
            project_id: binding.project_id,
            context_id: binding.context_id,
            revision,
            authority: self.authority_name().into(),
            label: if redo { "Redo" } else { "Undo" }.into(),
            changed,
            created: Vec::new(),
            dry_run: false,
            notices: self.take_notices(),
        })
    }

    /// Whether the agent's own last edit is the one undo would take back.
    pub fn agent_can_undo(&self, snapshot: &Snapshot) -> bool {
        self.own_undo
            .last()
            .is_some_and(|own| own.context == snapshot.context_id && own.after == snapshot.revision)
    }

    pub fn history(&mut self) -> Result<(Vec<String>, Vec<String>), ToolError> {
        self.check_handover()?;
        let response = self.send(AgentOperation::History, None)?;
        match self.answer(response)? {
            (_, AgentResult::Entries { past, future }) => Ok((past, future)),
            _ => Err(ToolError::unavailable(
                "The authority answered something else",
            )),
        }
    }

    /// The document as its owner sees it, with the project's revision.
    pub fn document(&mut self) -> Result<(u64, DocumentInfo), ToolError> {
        self.ready()?;
        self.check_handover()?;
        match &self.backend {
            Backend::Empty => Err(ToolError::no_project()),
            Backend::Local(local) => {
                let mut info = headless::info(&local.editor, local.document.as_ref());
                info.conflicted = local.conflicted;
                Ok((local.editor.revision(), info))
            }
            Backend::Desktop(_) => {
                let response = self.send(AgentOperation::Document, None)?;
                match self.answer(response)? {
                    (revision, AgentResult::Document { document }) => {
                        if let Some(binding) = &self.binding
                            && (binding.project_id != document.project_id
                                || binding.context_id != document.context_id)
                        {
                            return Err(ToolError::new(
                                codes::PROJECT_CHANGED,
                                "The window now holds another project; call desktop {op: \
                                 attach} to edit it",
                            ));
                        }
                        Ok((revision, *document))
                    }
                    _ => Err(ToolError::unavailable("The window answered something else")),
                }
            }
        }
    }

    /// Saves the document, or saves it as a new file.
    pub fn save(&mut self, destination: Option<PathBuf>) -> Result<DocumentInfo, ToolError> {
        // Save As writes a new file, which no window has open; it is how
        // work the window's file does not hold is kept.
        if destination.is_none() {
            self.check_handover()?;
        }
        let response = self.send(AgentOperation::Save { destination }, None)?;
        let (_, result) = self.answer(response)?;
        let AgentResult::Document { document } = result else {
            return Err(ToolError::unavailable(
                "The authority answered something else",
            ));
        };
        if let Backend::Local(local) = &mut self.backend
            && let Some(document) = &mut local.document
        {
            // A project saved for the first time now has a file to protect.
            document.claim()?;
        }
        if let Some(binding) = &mut self.binding {
            binding.path = document.path.clone();
        }
        Ok(*document)
    }

    // ---- Opening and closing --------------------------------------------

    /// Lets go of the project. A Local file that holds all its work loses
    /// its recovery copy (it is saved after every edit) and its lease; work
    /// that is in no file keeps its recovery copy. A window is left as it
    /// is. Exports this server started keep running.
    pub fn close(&mut self, discard: bool) -> Result<(), ToolError> {
        if !discard {
            self.check_leavable()?;
        }
        // Letting go is a choice: a server started for a window does not
        // follow it back on its own.
        self.following = false;
        let backend = std::mem::replace(&mut self.backend, Backend::Empty);
        self.binding = None;
        self.own_undo.clear();
        self.own_redo.clear();
        if let Backend::Local(local) = backend {
            self.let_go(local);
        }
        Ok(())
    }

    /// Refuses to leave a Local project that holds work no file has: a
    /// project never saved, recovered work, or an edit whose save failed.
    fn check_leavable(&self) -> Result<(), ToolError> {
        let Backend::Local(local) = &self.backend else {
            return Ok(());
        };
        if !local.editor.is_dirty() {
            return Ok(());
        }
        Err(
            if local
                .document
                .as_ref()
                .and_then(HeadlessDocument::path)
                .is_none()
            {
                ToolError::invalid(
                    "This project holds work that is in no file and would be lost; save it \
                     with project_save {destination}, or pass discard: true to project_close",
                )
            } else {
                ToolError::invalid(
                    "This project holds work its file does not have (recovered work, or an \
                     edit whose save was refused); keep it with project_save, or project_save \
                     {destination} for a new file. project_close {discard: true} leaves it in \
                     the recovery copy.",
                )
            },
        )
    }

    /// Gives up a Local project's file: its recovery copy goes when the file
    /// holds everything, and stays when it holds work no file has.
    fn let_go(&mut self, local: Local) {
        let Some(document) = local.document else {
            return;
        };
        if local.editor.is_dirty() {
            document.abandon();
            self.notices.push(
                "Work that is in no file was left in the project's recovery copy; opening the \
                 project with recover: true brings it back"
                    .into(),
            );
        } else if let Err(error) = document.release() {
            self.notices.push(format!(
                "The server's recovery copy could not be removed: {error}"
            ));
        }
    }

    fn adopt_local(&mut self, editor: Editor, document: Option<HeadlessDocument>) {
        self.binding = Some(Binding {
            project_id: editor.project().id,
            context_id: editor.instance_id(),
            path: document
                .as_ref()
                .and_then(|d| d.path().map(Path::to_path_buf)),
        });
        self.backend = Backend::Local(Local {
            editor,
            document,
            conflicted: false,
        });
        self.own_undo.clear();
        self.own_redo.clear();
        self.seen = None;
    }

    /// A new project, in a new file at `path` or in memory.
    pub fn create(&mut self, project: Project, path: Option<PathBuf>) -> Result<(), ToolError> {
        self.check_leavable()?;
        let mut editor = Editor::new(project)
            .map_err(|error| ToolError::invalid(format!("The project is not valid: {error}")))?;
        let document = match path {
            Some(path) => {
                let (mut document, recovery) = headless::create_document(&mut editor, &path)?;
                document.claim()?;
                if let Some(error) = recovery {
                    self.notices
                        .push(format!("The recovery copy could not be written: {error}"));
                }
                Some(document)
            }
            None => None,
        };
        // The project open until now goes only once the new one exists.
        self.close(true)?;
        self.adopt_local(editor, document);
        Ok(())
    }

    /// Opens `path` on the server, or attaches to the window that has it
    /// open with the same contents.
    pub fn open(&mut self, path: &Path, recover: bool) -> Result<(), ToolError> {
        self.following = false;
        if recover && !editor_store::recovery_path(path).exists() {
            return Err(ToolError::not_found(format!(
                "There is no recovery copy of {}: it was saved, or opened since",
                path.display()
            )));
        }
        let current = self
            .binding
            .as_ref()
            .and_then(|binding| binding.path.clone());
        let same = current
            .as_deref()
            .is_some_and(|current| same_file(current, path));
        if same && recover {
            return Err(ToolError::invalid(format!(
                "{} is open in this server already; its recovery copy is this server's own",
                path.display()
            )));
        }
        if same && !recover {
            let conflicted = matches!(&self.backend, Backend::Local(local) if local.conflicted);
            if !conflicted {
                // Already open here: only a window that took it since matters.
                return self.check_handover();
            }
            // The file changed under the server: it is read again, and what
            // is on disk now is what is edited.
            self.close(false)?;
        }
        self.check_leavable()?;
        // A window that has the file open is where the project is, with the
        // person's unsaved work if there is any: edits go there.
        if !recover {
            let holders: Vec<Session> = self
                .discover(true)
                .into_iter()
                .filter(|session| {
                    session
                        .document
                        .path
                        .as_deref()
                        .is_some_and(|theirs| same_file(theirs, path))
                })
                .collect();
            match holders.as_slice() {
                [] => {}
                [holder] => {
                    self.close(false)?;
                    self.become_desktop(holder);
                    self.notices.push(format!(
                        "{} is open in an Aster window, so edits go to the window and appear \
                         there at once; they are saved when the window saves (project_save).",
                        path.display()
                    ));
                    return Ok(());
                }
                _ => {
                    return Err(ToolError::new(
                        codes::DESKTOP_CONFLICT,
                        format!(
                            "{} is open in more than one Aster window; close all but one",
                            path.display()
                        ),
                    ));
                }
            }
        }
        let (editor, mut document) = headless::open_headless(path, false, recover)?;
        // An editor that stopped without saving left its work in a recovery
        // copy, which this server's first edit would replace: it is opened
        // only when asked for.
        if !recover && editor_store::recovery_path(path).exists() {
            let differs = editor_store::load_recovery(path)
                .map_or(true, |kept| kept.project() != editor.project());
            if differs {
                return Err(ToolError::new(
                    codes::RECOVERY_AVAILABLE,
                    format!(
                        "{} has a recovery copy with unsaved work from an editor that stopped. \
                         Open it with recover: true (project_save then keeps it), or ask the \
                         person; the copy is left as it is.",
                        path.display()
                    ),
                ));
            }
        }
        // The project open until now goes only once the new one is held.
        if let Err(error) = document.claim() {
            return Err(match error {
                editor_store::StoreError::RecoveryInUse(_) => ToolError::new(
                    codes::PROJECT_BUSY,
                    format!(
                        "Another editor has {} open with unsaved work (an Aster window or \
                         another server). If it is a window, use desktop {{op: attach}}.",
                        path.display()
                    ),
                ),
                other => other.into(),
            });
        }
        self.close(true)?;
        self.adopt_local(editor, Some(document));
        self.check_handover()
    }

    /// Edits the project open in a window from now on.
    pub fn attach(&mut self, instance: Option<Uuid>) -> Result<Session, ToolError> {
        let sessions = self.discover(true);
        // A server started for a window goes back to it when no window
        // is named.
        let home = self.home.as_ref().map(|file| {
            sessions
                .iter()
                .find(|session| &session.file == file)
                .map(|session| session.descriptor.instance_id)
        });
        let instance = match (instance, home) {
            (Some(named), _) => Some(named),
            (None, Some(Some(own))) => Some(own),
            (None, Some(None)) => {
                return Err(ToolError::new(
                    codes::DESKTOP_CLOSED,
                    "The Aster window this agent was started in did not answer; it may have \
                     closed",
                ));
            }
            (None, None) => None,
        };
        let going_home = matches!(home, Some(Some(own)) if instance == Some(own));
        let local_path = self.binding.as_ref().and_then(|b| b.path.clone());
        let chosen: Vec<&Session> = sessions
            .iter()
            .filter(|session| match instance {
                Some(id) => session.descriptor.instance_id == id,
                None => match &local_path {
                    Some(path) => session
                        .document
                        .path
                        .as_deref()
                        .is_some_and(|p| same_file(p, path)),
                    None => true,
                },
            })
            .collect();
        let session = match chosen.as_slice() {
            [one] => (*one).clone(),
            [] => {
                return Err(ToolError::not_found(
                    "No Aster window answered. Open the project in Aster (or call desktop {op: \
                     open}) and try again; desktop {op: list} shows the windows found.",
                ));
            }
            _ => {
                return Err(ToolError::new(
                    codes::DESKTOP_CONFLICT,
                    "More than one Aster window is open; name one with instance_id from \
                     desktop {op: list}",
                ));
            }
        };
        self.check_leavable()?;
        if let Some(path) = &session.document.path
            && !self.roots.contains_path(path)
        {
            return Err(ToolError::new(
                codes::OUTSIDE_ROOTS,
                "The window's project is outside the folders this server may use; the person \
                 can add one with --root, or for Aster's own agent in Settings \u{203a} Agent \u{203a} \
                 Folders",
            ));
        }
        self.become_desktop(&session);
        self.following = going_home;
        Ok(session)
    }

    fn become_desktop(&mut self, session: &Session) {
        if let Backend::Local(local) = std::mem::replace(&mut self.backend, Backend::Empty) {
            self.let_go(local);
        }
        self.backend = Backend::Desktop(Desktop {
            descriptor: session.descriptor.clone(),
        });
        self.binding = Some(Binding {
            project_id: session.document.project_id,
            context_id: session.document.context_id,
            path: session.document.path.clone(),
        });
        self.own_undo.clear();
        self.own_redo.clear();
    }

    /// Before a Local project is used: a window that has its file open
    /// takes it over when it holds the same version, and makes the server
    /// refuse otherwise. Never two writers.
    fn check_handover(&mut self) -> Result<(), ToolError> {
        let Backend::Local(local) = &self.backend else {
            return Ok(());
        };
        let Some(path) = local
            .document
            .as_ref()
            .and_then(|d| d.path().map(Path::to_path_buf))
        else {
            return Ok(());
        };
        let ours = local
            .document
            .as_ref()
            .and_then(HeadlessDocument::file_version);
        let unsaved = local.editor.is_dirty();
        let sessions = self.discover(false);
        let holders: Vec<Session> = sessions
            .into_iter()
            .filter(|session| {
                session
                    .document
                    .path
                    .as_deref()
                    .is_some_and(|theirs| same_file(theirs, &path))
            })
            .collect();
        match holders.as_slice() {
            [] => Ok(()),
            [_] if unsaved => Err(ToolError::new(
                codes::DESKTOP_CONFLICT,
                format!(
                    "{} is open in an Aster window, and this server holds work for it that the \
                     file does not have. Keep that work in a new file with project_save \
                     {{destination}}.",
                    path.display()
                ),
            )),
            [holder] if holder.document.file_version == ours && ours.is_some() => {
                self.become_desktop(holder);
                self.notices.push(format!(
                    "{} is open in an Aster window, so edits now go to the window and appear \
                     there at once; they are saved when the window saves (project_save). The \
                     server's own undo history for this project ended here.",
                    path.display()
                ));
                Ok(())
            }
            [holder] if !holder.document.dirty => Err(ToolError::new(
                codes::DESKTOP_STALE,
                format!(
                    "{} is open in an Aster window that shows an older version of it. Reopen \
                     it in the window, or close it there, then retry.",
                    path.display()
                ),
            )),
            [_] => Err(ToolError::new(
                codes::DESKTOP_CONFLICT,
                format!(
                    "{} is open in an Aster window with unsaved changes made from a different \
                     version. Ask the person to save or close it, or call desktop {{op: \
                     attach}} to continue with what the window shows.",
                    path.display()
                ),
            )),
            _ => Err(ToolError::new(
                codes::DESKTOP_CONFLICT,
                format!(
                    "{} is open in more than one Aster window; close all but one",
                    path.display()
                ),
            )),
        }
    }

    /// The windows that answer, looked for at most once a second unless
    /// `fresh`.
    pub fn discover(&mut self, fresh: bool) -> Vec<Session> {
        if !fresh
            && let Some((when, sessions)) = &self.seen
            && when.elapsed() < DISCOVERY_EVERY
        {
            return sessions.clone();
        }
        let mut files: Vec<PathBuf> = self.launched.clone();
        if let Some(folder) = &self.sessions
            && let Ok(entries) = std::fs::read_dir(folder)
        {
            files.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                    .take(MOST_SESSIONS),
            );
        }
        let mut seen = HashSet::new();
        let mut sessions = Vec::new();
        for file in files {
            if !seen.insert(file.clone()) {
                continue;
            }
            let Some(descriptor) = std::fs::read(&file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<SessionDescriptor>(&bytes).ok())
            else {
                continue;
            };
            self.requests += 1;
            let request = AgentRequest {
                request_id: format!("{}-{}", self.tag, self.requests),
                expected_revision: None,
                expected_project_id: None,
                expected_context_id: None,
                operation: AgentOperation::Document,
            };
            let Ok(response) = editor_agent::connect_within(
                &descriptor,
                request,
                DISCOVERY_CONNECT,
                DISCOVERY_READ,
            ) else {
                continue;
            };
            if let AgentResult::Document { document } = response.result {
                sessions.push(Session {
                    file,
                    descriptor,
                    document: *document,
                });
            }
        }
        self.seen = Some((Instant::now(), sessions.clone()));
        sessions
    }

    /// Launches the `aster` window beside this server on the project's
    /// file and edits it there once it answers.
    pub fn launch_desktop(&mut self, path: Option<PathBuf>) -> Result<Session, ToolError> {
        self.following = false;
        if let Some(child) = &mut self.launched_window
            && child.try_wait().ok().flatten().is_none()
        {
            return Err(ToolError::new(
                codes::BUSY,
                "This server already launched a window that is still open; attach to it with \
                 desktop {op: attach}",
            ));
        }
        let path = match path {
            Some(path) => path,
            None => self
                .binding
                .as_ref()
                .and_then(|binding| binding.path.clone())
                .ok_or_else(|| {
                    ToolError::invalid(
                        "The project has no file to open in a window; save it first with \
                         project_save {destination}",
                    )
                })?,
        };
        self.check_leavable()?;
        let program = self.desktop_program.clone().ok_or_else(|| {
            ToolError::unavailable("No Aster program was found beside this server")
        })?;
        let folder = std::env::temp_dir().join(format!("aster-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&folder)
            .map_err(|error| ToolError::unavailable(format!("No session folder: {error}")))?;
        // Let go of the file first, so the window owns it from the start;
        // it is opened here again if the window cannot start.
        let reopen = match &self.backend {
            Backend::Local(local) => local
                .document
                .as_ref()
                .and_then(HeadlessDocument::path)
                .map(Path::to_path_buf),
            _ => None,
        };
        if matches!(self.backend, Backend::Local(_)) {
            self.close(false)?;
        }
        let file = folder.join(format!("{}.json", Uuid::new_v4()));
        let mut command = std::process::Command::new(&program);
        command.arg("--agent-session").arg(&file);
        if let Some(settings) = &self.desktop_settings {
            command.arg("--settings").arg(settings);
        }
        command
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env_remove("ASTER_MCP_TOKEN");
        detach(&mut command);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                if let Some(path) = reopen
                    && let Err(again) = self.open(&path, false)
                {
                    self.notices.push(format!(
                        "{} could not be opened here again: {}",
                        path.display(),
                        again.message
                    ));
                }
                return Err(ToolError::unavailable(format!(
                    "{} could not start: {error}",
                    program.display()
                )));
            }
        };
        self.launched_window = Some(child);
        self.launched.push(file.clone());
        let until = Instant::now() + Duration::from_secs(45);
        while Instant::now() < until {
            if let Some(child) = &mut self.launched_window
                && let Ok(Some(status)) = child.try_wait()
            {
                return Err(ToolError::unavailable(format!(
                    "The Aster window ended before it opened the project ({status})"
                )));
            }
            if file.exists() {
                let found = self.discover(true).into_iter().find(|session| {
                    session.file == file
                        && session
                            .document
                            .path
                            .as_deref()
                            .is_some_and(|p| same_file(p, &path))
                });
                if let Some(session) = found {
                    self.become_desktop(&session);
                    return Ok(session);
                }
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err(ToolError::unavailable(
            "The Aster window did not open the project within 45 seconds; when it has, call \
             desktop {op: attach}",
        ))
    }
}

impl Workspace {
    /// Before the process ends: exports stop, and the project's lease and
    /// recovery copy go (its file is saved after every edit).
    pub fn shut_down(&mut self) {
        if let Some(queue) = self.renders.take() {
            queue.shutdown();
        }
        if let Some(mut child) = self.launched_window.take() {
            // The window is the person's now; it is not waited for.
            let _ = child.try_wait();
        }
        if let Backend::Local(local) = std::mem::replace(&mut self.backend, Backend::Empty) {
            self.let_go(local);
        }
        self.binding = None;
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // Saved after every edit: the recovery copy holds only history, which
        // must not be offered to the next window as unsaved work. Work that
        // is in no file stays in it.
        if let Backend::Local(local) = std::mem::replace(&mut self.backend, Backend::Empty) {
            self.let_go(local);
        }
    }
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_: &mut std::process::Command) {}

/// Two names of one file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Media a command adds must be inside the roots, or already in the project.
/// Media new to the project must be a kind the editor imports, and is probed
/// again: what a command says it is is not taken on trust, so a playlist or
/// a list of other files cannot come in as media.
pub fn check_paths(roots: &Roots, command: &Command, project: &Project) -> Result<(), ToolError> {
    let known = |path: &Path| project.assets.values().any(|asset| asset.path == path);
    let probed = |asset: &editor_core::Asset| -> Result<(), ToolError> {
        if !editor_media::inspect::importable(&asset.path) {
            return Err(ToolError::invalid(format!(
                "{} is not a kind of media Aster imports",
                asset.path.display()
            )));
        }
        let never = std::sync::atomic::AtomicBool::new(false);
        let imported =
            editor_media::inspect::import_now(vec![asset.path.clone()], &never, &|path| {
                roots.contains(path)
            });
        match (imported.assets.first(), imported.errors.first()) {
            (Some(found), _) if found.kind == asset.kind => Ok(()),
            (Some(found), _) => Err(ToolError::invalid(format!(
                "{} is {:?} media, not {:?}; import it with media_import",
                asset.path.display(),
                found.kind,
                asset.kind
            ))),
            (None, Some(error)) => Err(ToolError::invalid(format!(
                "{} is not media Aster can use: {}",
                asset.path.display(),
                error.message
            ))),
            (None, None) => Err(ToolError::invalid(format!(
                "{} is not media Aster can use",
                asset.path.display()
            ))),
        }
    };
    let refuse = |path: &Path| {
        ToolError::new(
            codes::OUTSIDE_ROOTS,
            format!(
                "{} is not a file inside the folders this server may use; import media with \
                 media_import",
                path.display()
            ),
        )
    };
    match command {
        Command::AddAssets { assets } => {
            for asset in assets.iter().filter(|asset| !known(&asset.path)) {
                if !roots.readable(&asset.path) {
                    return Err(refuse(&asset.path));
                }
                probed(asset)?;
            }
        }
        Command::RestoreProject { project: restored } => {
            for asset in restored.assets.values().filter(|asset| !known(&asset.path)) {
                if !roots.readable(&asset.path) {
                    return Err(refuse(&asset.path));
                }
                probed(asset)?;
            }
        }
        Command::Batch { commands, .. } => {
            for command in commands {
                check_paths(roots, command, project)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Clips that overlap on a track after an edit and did not before.
#[derive(Debug, Clone, Serialize)]
pub struct Overlap {
    pub track_id: Uuid,
    pub clips: [Uuid; 2],
}

pub fn new_overlaps(before: &Project, after: &Project) -> Vec<Overlap> {
    let pairs = |project: &Project| {
        let mut pairs = Vec::new();
        for track in &project.tracks {
            let mut latest: Option<&editor_core::Clip> = None;
            for clip in &track.clips {
                if let Some(previous) = latest
                    && clip.start < previous.end()
                {
                    pairs.push(Overlap {
                        track_id: track.id,
                        clips: [previous.id, clip.id],
                    });
                }
                if latest.is_none_or(|previous| clip.end() > previous.end()) {
                    latest = Some(clip);
                }
            }
        }
        pairs
    };
    let old: HashSet<[Uuid; 2]> = pairs(before).into_iter().map(|pair| pair.clips).collect();
    pairs(after)
        .into_iter()
        .filter(|pair| !old.contains(&pair.clips))
        .collect()
}

/// A task run on the workspace's thread.
type Task = Box<dyn FnOnce(&mut Workspace) + Send>;

/// The way to the workspace: a bounded queue to the thread that owns it.
#[derive(Clone)]
pub struct Authority {
    tasks: SyncSender<Task>,
}

impl Authority {
    pub fn start(workspace: Workspace) -> Self {
        let (tasks, inbox) = sync_channel::<Task>(64);
        thread::Builder::new()
            .name("aster-mcp-authority".into())
            .spawn(move || {
                let mut workspace = workspace;
                for task in inbox {
                    // A task that panics loses its answer, not the project.
                    let _ = catch_unwind(AssertUnwindSafe(|| task(&mut workspace)));
                }
            })
            .expect("the authority thread starts");
        Self { tasks }
    }

    /// Runs `work` on the workspace and waits for its answer without
    /// holding a runtime thread.
    pub async fn run<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Workspace) -> Result<R, ToolError> + Send + 'static,
    ) -> Result<R, ToolError> {
        let (answer, answered) = tokio::sync::oneshot::channel();
        let task: Task = Box::new(move |workspace| {
            let _ = answer.send(work(workspace));
        });
        match self.tasks.try_send(task) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(ToolError::new(
                    codes::BUSY,
                    "The server has too many requests waiting; retry",
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(ToolError::unavailable("The server is stopping"));
            }
        }
        answered
            .await
            .map_err(|_| ToolError::unavailable("The request failed inside the server"))?
    }
}
