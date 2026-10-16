//! Local, authenticated agent transport. All edits still enter the core reducer.
mod ai;
mod context;
pub mod conversation;
mod host;
mod planner;
mod provider;
pub use ai::*;
pub use conversation::{Conversation, ToolServer, TurnEvent};
use editor_core::export::{Capabilities, ExportError, ExportPreset, ExportSettings, RenderJobInfo};
use editor_core::{Command, Editor, Project};
pub use host::*;
pub use planner::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
/// The largest request a session reads: room for a whole project restored
/// from a checkpoint or a long transcript's captions in one edit.
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024;

/// Error codes a client can match on.
pub mod codes {
    pub const REVISION_REQUIRED: &str = "revision_required";
    pub const STALE_REVISION: &str = "stale_revision";
    pub const PROJECT_REQUIRED: &str = "project_required";
    pub const STALE_PROJECT: &str = "stale_project";
    pub const CONTEXT_REQUIRED: &str = "context_required";
    pub const STALE_CONTEXT: &str = "stale_context";
    pub const INVALID_EDIT: &str = "invalid_edit";
    /// The desktop is still finding out what this host's FFmpeg can encode.
    pub const CAPABILITIES_PENDING: &str = "capabilities_pending";
    /// The export settings contradict themselves, the limits or the destination.
    pub const INVALID_EXPORT: &str = "invalid_export";
    /// This host's FFmpeg cannot make what was asked.
    pub const UNSUPPORTED_EXPORT: &str = "unsupported_export";
    /// The range holds nothing to export.
    pub const EMPTY_EXPORT: &str = "empty_export";
    pub const QUEUE_FULL: &str = "queue_full";
    /// A file is at the destination and `overwrite` was not set.
    pub const DESTINATION_EXISTS: &str = "destination_exists";
    /// No job has that id, or the job cannot do what was asked in its state.
    pub const UNKNOWN_JOB: &str = "unknown_job";
    /// The session cannot run this operation; render jobs need the desktop.
    pub const UNSUPPORTED: &str = "unsupported";
    /// `save` without a destination, for a project that has no file.
    pub const NO_DESTINATION: &str = "no_destination";
    /// A save is already being written or prepared.
    pub const SAVE_BUSY: &str = "save_busy";
    /// The person is in the middle of a change (a drag, a focused field);
    /// nothing was applied. Retry once the gesture ends.
    pub const PERSON_EDITING: &str = "person_editing";
    /// The window is asking the person something, or writing a save;
    /// nothing was applied. Retry later.
    pub const DESKTOP_BUSY: &str = "desktop_busy";
}

/// Every operation name, as `operation` spells it.
pub const OPERATIONS: [&str; 17] = [
    "inspect",
    "apply",
    "undo",
    "redo",
    "capabilities",
    "export_capabilities",
    "queue_export",
    "render_jobs",
    "cancel_render",
    "pause_render",
    "resume_render",
    "retry_render",
    "remove_render",
    "history",
    "document",
    "save",
    "changes_since",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub request_id: String,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub expected_project_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub expected_context_id: Option<uuid::Uuid>,
    #[serde(flatten)]
    pub operation: AgentOperation,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum AgentOperation {
    Inspect,
    Apply {
        command: Box<Command>,
    },
    Undo,
    Redo,
    Capabilities,
    /// What this host can encode, the presets and the settings schema.
    ExportCapabilities,
    /// Queues an export of the project as it stands at `expected_revision`.
    QueueExport {
        settings: ExportSettings,
        /// An absolute path; an existing file is replaced only with `overwrite`.
        destination: PathBuf,
        #[serde(default)]
        overwrite: bool,
    },
    RenderJobs,
    CancelRender {
        job: Uuid,
    },
    PauseRender {
        job: Uuid,
    },
    ResumeRender {
        job: Uuid,
    },
    RetryRender {
        job: Uuid,
    },
    RemoveRender {
        job: Uuid,
    },
    /// The labels of the undo and redo entries.
    History,
    /// Compact changes in the current editing context, including undo and redo.
    ChangesSince {
        revision: u64,
    },
    /// The document as the session that owns it sees it: its file, whether
    /// it is saved, and the playhead and selection of a window.
    Document,
    /// Saves the document to its file, or to `destination`, a new file.
    /// Needs the project and context guards; the revision saved is the
    /// current one.
    Save {
        #[serde(default)]
        destination: Option<PathBuf>,
    },
}
impl AgentOperation {
    /// Operations answered from the desktop's render queue, never by `handle`.
    pub fn is_render(&self) -> bool {
        matches!(
            self,
            Self::ExportCapabilities
                | Self::QueueExport { .. }
                | Self::RenderJobs
                | Self::CancelRender { .. }
                | Self::PauseRender { .. }
                | Self::ResumeRender { .. }
                | Self::RetryRender { .. }
                | Self::RemoveRender { .. }
        )
    }

    /// Operations that change the project and so need every guard.
    pub fn is_mutation(&self) -> bool {
        matches!(self, Self::Apply { .. } | Self::Undo | Self::Redo)
    }

    /// Operations answered by the session that owns the document (the
    /// desktop window or a headless host), never by `handle`.
    pub fn is_host(&self) -> bool {
        matches!(self, Self::Document | Self::Save { .. })
    }
}

/// What the session that owns a document knows about it beyond the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DocumentInfo {
    pub project_id: Uuid,
    pub context_id: Uuid,
    pub name: String,
    /// The project file; `None` until the project is first saved.
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// A save or a Save As is being written or prepared.
    #[serde(default)]
    pub saving: bool,
    /// The file changed on disk since this session read or saved it, so
    /// saving to it is refused until someone decides.
    #[serde(default)]
    pub conflicted: bool,
    /// The window's playhead; `None` for a headless session.
    #[serde(default)]
    pub playhead: Option<editor_core::Time>,
    /// The clips selected in the window.
    #[serde(default)]
    pub selection: Vec<Uuid>,
    #[serde(default)]
    pub can_undo: bool,
    #[serde(default)]
    pub can_redo: bool,
    /// The version of the file this session last read or saved, as
    /// `bytes:sha256`; `None` without a file. Two sessions with the same
    /// version hold the same file contents.
    #[serde(default)]
    pub file_version: Option<String>,
    /// The person holds a change that is not yet an edit: a drag, a
    /// focused field. Mutations are refused `person_editing` meanwhile.
    #[serde(default)]
    pub editing: bool,
    /// The window is asking the person something (unsaved work, a
    /// conflict). Mutations are refused `desktop_busy` meanwhile.
    #[serde(default)]
    pub asking: bool,
    /// Counts finished saves; with `last_save`, tells a save asked for
    /// from one finished since.
    #[serde(default)]
    pub save_count: u64,
    /// How the last finished save ended.
    #[serde(default)]
    pub last_save: Option<SaveOutcome>,
}

/// How a save ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SaveOutcome {
    pub ok: bool,
    pub path: PathBuf,
    /// Why it failed; empty when it did not.
    #[serde(default)]
    pub message: String,
}

/// The IDs `after` has that `before` does not: tracks, clips, assets,
/// folders, markers and trackers, in that order.
pub fn created(before: &Project, after: &Project) -> Vec<Uuid> {
    use std::collections::HashSet;
    let ids = |project: &Project| -> Vec<Uuid> {
        let mut ids = Vec::new();
        for track in &project.tracks {
            ids.push(track.id);
        }
        for track in &project.tracks {
            ids.extend(track.clips.iter().map(|clip| clip.id));
        }
        ids.extend(project.assets.keys());
        ids.extend(project.folders.keys());
        ids.extend(project.markers.iter().map(|marker| marker.id));
        ids.extend(project.trackers.keys());
        ids
    };
    if std::ptr::eq(before, after) {
        return Vec::new();
    }
    let known: HashSet<Uuid> = ids(before).into_iter().collect();
    ids(after)
        .into_iter()
        .filter(|id| !known.contains(id))
        .collect()
}
impl DocumentInfo {
    /// The parts every host knows from its editor alone.
    pub fn of(editor: &Editor, path: Option<PathBuf>) -> Self {
        Self {
            project_id: editor.project().id,
            context_id: editor.instance_id(),
            name: editor.project().name.clone(),
            path,
            dirty: editor.is_dirty(),
            saving: false,
            conflicted: false,
            playhead: None,
            selection: Vec::new(),
            can_undo: editor.can_undo(),
            can_redo: editor.can_redo(),
            file_version: None,
            editing: false,
            asking: false,
            save_count: 0,
            last_save: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentResponse {
    pub request_id: String,
    pub revision: u64,
    pub result: AgentResult,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentResult {
    Project {
        context_id: uuid::Uuid,
        project: Arc<Project>,
        dirty: bool,
        can_undo: bool,
        can_redo: bool,
    },
    Applied {
        label: String,
        changed: bool,
        /// The IDs the edit added: tracks, clips, assets, folders, markers
        /// and trackers. Absent from older sessions.
        #[serde(default)]
        created: Vec<Uuid>,
    },
    History {
        changed: bool,
    },
    Capabilities {
        protocol_version: u32,
        /// Every operation name; absent from older sessions.
        #[serde(default)]
        operations: Vec<String>,
        commands: Vec<String>,
        command_schema: serde_json::Value,
    },
    /// Boxed, as is `RenderJob`'s job, so a result stays small to move.
    ExportCapabilities {
        capabilities: Box<Capabilities>,
        presets: Vec<ExportPreset>,
        settings_schema: serde_json::Value,
    },
    RenderJobs {
        jobs: Vec<RenderJobInfo>,
    },
    RenderJob {
        job: Box<RenderJobInfo>,
    },
    /// Undo labels oldest first, redo labels next redo first.
    Entries {
        past: Vec<String>,
        future: Vec<String>,
        #[serde(default)]
        past_entries: Vec<editor_core::ChangeSummary>,
        #[serde(default)]
        future_entries: Vec<editor_core::ChangeSummary>,
        #[serde(default)]
        truncated: bool,
    },
    Changes {
        project_id: Uuid,
        context_id: Uuid,
        changes: editor_core::ChangesSince,
    },
    Document {
        document: Box<DocumentInfo>,
    },
    Error {
        code: String,
        message: String,
    },
}
impl AgentResult {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }

    /// A refused export plan as the agent sees it.
    pub fn export_error(error: &ExportError) -> Self {
        let code = match error {
            ExportError::Invalid(_) => codes::INVALID_EXPORT,
            ExportError::Unsupported(_) => codes::UNSUPPORTED_EXPORT,
            ExportError::Empty(_) => codes::EMPTY_EXPORT,
        };
        Self::error(code, error.to_string())
    }
}
impl AgentResponse {
    pub fn error(
        request_id: String,
        revision: u64,
        code: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            revision,
            result: AgentResult::error(code, message),
        }
    }
}

/// Exact serialized schema of `ExportSettings`, for `queue_export`.
pub fn export_settings_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ExportSettings)).unwrap_or(serde_json::Value::Null)
}

/// The guards every mutation passes: the request names the revision, project
/// and editing context it was planned against, and they are the editor's.
pub fn check_target(editor: &Editor, request: &AgentRequest) -> Result<(), AgentResult> {
    if request.expected_revision.is_none() {
        Err(AgentResult::error(
            codes::REVISION_REQUIRED,
            "Inspect the session and include expected_revision for every mutation",
        ))
    } else if request.expected_revision != Some(editor.revision()) {
        Err(AgentResult::error(
            codes::STALE_REVISION,
            format!(
                "Expected revision {:?}; current {}",
                request.expected_revision,
                editor.revision()
            ),
        ))
    } else if request.expected_project_id.is_none() {
        Err(AgentResult::error(
            codes::PROJECT_REQUIRED,
            "Inspect the session and include expected_project_id for every mutation",
        ))
    } else if request.expected_project_id != Some(editor.project().id) {
        Err(AgentResult::error(
            codes::STALE_PROJECT,
            "The session now contains a different project; inspect before editing",
        ))
    } else if request.expected_context_id.is_none() {
        Err(AgentResult::error(
            codes::CONTEXT_REQUIRED,
            "Include expected_context_id from inspect for every mutation",
        ))
    } else if request.expected_context_id != Some(editor.instance_id()) {
        Err(AgentResult::error(
            codes::STALE_CONTEXT,
            "The project was reloaded; inspect the new editing context before editing",
        ))
    } else {
        Ok(())
    }
}

/// The guards of `save`: the request names the project and editing context
/// it means. An explicit revision is checked; otherwise the current one is saved.
pub fn check_identity(editor: &Editor, request: &AgentRequest) -> Result<(), AgentResult> {
    let mut request = request.clone();
    request.expected_revision = request.expected_revision.or(Some(editor.revision()));
    check_target(editor, &request)
}

/// The checks a `queue_export` passes before the desktop plans it: the
/// mutation guards, so the export is of the project the agent inspected, and
/// a destination that is absolute, names a file in an existing folder and is
/// free unless `overwrite` is set. Reads the destination's metadata.
pub fn check_queue_export(editor: &Editor, request: &AgentRequest) -> Result<(), AgentResult> {
    let AgentOperation::QueueExport {
        destination,
        overwrite,
        ..
    } = &request.operation
    else {
        return Err(AgentResult::error(
            codes::INVALID_EXPORT,
            "Only a queue_export request can be queued",
        ));
    };
    check_target(editor, request)?;
    let invalid = |message: String| Err(AgentResult::error(codes::INVALID_EXPORT, message));
    if !destination.is_absolute() {
        return invalid(format!(
            "The destination {} must be an absolute path",
            destination.display()
        ));
    }
    let (Some(folder), Some(_)) = (destination.parent(), destination.file_name()) else {
        return invalid(format!(
            "The destination {} does not name a file",
            destination.display()
        ));
    };
    if !folder.is_dir() {
        return invalid(format!(
            "The destination folder {} does not exist",
            folder.display()
        ));
    }
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_dir() => invalid(format!(
            "The destination {} is a folder",
            destination.display()
        )),
        Ok(_) if !overwrite => Err(AgentResult::error(
            codes::DESTINATION_EXISTS,
            format!(
                "{} exists; send overwrite: true to replace it",
                destination.display()
            ),
        )),
        _ => Ok(()),
    }
}

/// Desktop and headless sessions call this same function on their owning thread.
/// Mutation requests target both project identity and revision, so opening a
/// replacement project cannot retarget a queued or stale agent plan.
pub fn handle(editor: &mut Editor, request: AgentRequest) -> AgentResponse {
    let guard = if request.operation.is_mutation() {
        check_target(editor, &request).err()
    } else if matches!(
        &request.operation,
        AgentOperation::History | AgentOperation::ChangesSince { .. }
    ) {
        // A journal belongs to one editing context. MCP binds these optional
        // read guards before its snapshot, so replacing/reloading the project
        // between requests cannot masquerade as an empty prior journal.
        if request
            .expected_project_id
            .is_some_and(|id| id != editor.project().id)
        {
            Some(AgentResult::error(
                codes::STALE_PROJECT,
                "The session now contains a different project; inspect before reading history",
            ))
        } else if request
            .expected_context_id
            .is_some_and(|id| id != editor.instance_id())
        {
            Some(AgentResult::error(
                codes::STALE_CONTEXT,
                "The project was reloaded; inspect the new editing context before reading history",
            ))
        } else {
            None
        }
    } else {
        None
    };
    let result = guard.unwrap_or_else(|| match request.operation {
        AgentOperation::Inspect => AgentResult::Project {
            context_id: editor.instance_id(),
            project: editor.snapshot(),
            dirty: editor.is_dirty(),
            can_undo: editor.can_undo(),
            can_redo: editor.can_redo(),
        },
        AgentOperation::Apply { command } => {
            let before = editor.snapshot();
            match editor.dispatch(*command) {
                Ok(outcome) => AgentResult::Applied {
                    label: outcome.label,
                    changed: outcome.changed,
                    created: created(&before, editor.project()),
                },
                Err(error) => AgentResult::error(codes::INVALID_EDIT, error.to_string()),
            }
        }
        AgentOperation::Undo => AgentResult::History {
            changed: editor.undo(),
        },
        AgentOperation::Redo => AgentResult::History {
            changed: editor.redo(),
        },
        AgentOperation::History => AgentResult::Entries {
            past: editor
                .history()
                .skip(editor.history().count().saturating_sub(50))
                .map(|entry| entry.label.chars().take(256).collect())
                .collect(),
            future: editor
                .future()
                .take(50)
                .map(|entry| entry.label.chars().take(256).collect())
                .collect(),
            past_entries: editor
                .history()
                .skip(editor.history().count().saturating_sub(50))
                .map(|entry| {
                    editor_core::summarize_changes(&entry.label, &entry.before, &entry.after)
                })
                .collect(),
            future_entries: editor
                .future()
                .take(50)
                .map(|entry| {
                    editor_core::summarize_changes(&entry.label, &entry.before, &entry.after)
                })
                .collect(),
            truncated: editor.history().count() > 50 || editor.future().count() > 50,
        },
        AgentOperation::ChangesSince { revision } => AgentResult::Changes {
            project_id: editor.project().id,
            context_id: editor.instance_id(),
            changes: editor.changes_since(revision),
        },
        AgentOperation::Document | AgentOperation::Save { .. } => AgentResult::error(
            codes::UNSUPPORTED,
            "The session that owns the document answers this; this one has no document",
        ),
        AgentOperation::Capabilities => AgentResult::Capabilities {
            protocol_version: PROTOCOL_VERSION,
            operations: OPERATIONS.into_iter().map(str::to_owned).collect(),
            command_schema: command_schema(),
            commands: [
                "add_assets",
                "remove_assets",
                "add_folder",
                "move_asset_to_folder",
                "add_track",
                "remove_track",
                "reorder_track",
                "set_track_state",
                "add_clip",
                "patch_clip",
                "delete_clips",
                "move_clips",
                "trim_clip",
                "split_clip",
                "set_transform",
                "set_playback_rate",
                "set_gain",
                "set_fades",
                "set_text",
                "set_text_style",
                "set_text_animation",
                "set_words",
                "set_graphic",
                "set_track_language",
                "copy_captions",
                "set_effects",
                "set_masks",
                "set_blend_mode",
                "set_keyframes",
                "set_retime",
                "set_transition",
                "apply_preset",
                "set_tracker",
                "remove_tracker",
                "set_follow",
                "link_clips",
                "unlink_clips",
                "group_clips",
                "ungroup_clips",
                "add_marker",
                "remove_marker",
                "rename_project",
                "set_settings",
                "restore_project",
                "batch",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        },
        AgentOperation::ExportCapabilities
        | AgentOperation::QueueExport { .. }
        | AgentOperation::RenderJobs
        | AgentOperation::CancelRender { .. }
        | AgentOperation::PauseRender { .. }
        | AgentOperation::ResumeRender { .. }
        | AgentOperation::RetryRender { .. }
        | AgentOperation::RemoveRender { .. } => AgentResult::error(
            codes::UNSUPPORTED,
            "Render jobs run in the desktop editor window; connect to its session to export",
        ),
    });
    AgentResponse {
        request_id: request.request_id,
        revision: editor.revision(),
        result,
    }
}
