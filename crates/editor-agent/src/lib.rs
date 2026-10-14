//! Local, authenticated agent transport. All edits still enter the core reducer.
mod ai;
mod context;
mod host;
mod planner;
mod provider;
pub use ai::*;
use editor_core::export::{Capabilities, ExportError, ExportPreset, ExportSettings, RenderJobInfo};
use editor_core::{Command, Editor, Project};
pub use host::*;
pub use planner::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

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
}

/// Every operation name, as `operation` spells it.
pub const OPERATIONS: [&str; 13] = [
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
        AgentOperation::Apply { command } => match editor.dispatch(*command) {
            Ok(outcome) => AgentResult::Applied {
                label: outcome.label,
                changed: outcome.changed,
            },
            Err(error) => AgentResult::error(codes::INVALID_EDIT, error.to_string()),
        },
        AgentOperation::Undo => AgentResult::History {
            changed: editor.undo(),
        },
        AgentOperation::Redo => AgentResult::History {
            changed: editor.redo(),
        },
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
                "delete_clips",
                "move_clips",
                "trim_clip",
                "split_clip",
                "set_transform",
                "set_playback_rate",
                "set_gain",
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
