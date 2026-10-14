//! Local, authenticated agent transport. All edits still enter the core reducer.
mod ai;
mod context;
mod host;
mod planner;
mod provider;
pub use ai::*;
use editor_core::{Command, Editor, Project};
pub use host::*;
pub use planner::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum AgentOperation {
    Inspect,
    Apply { command: Box<Command> },
    Undo,
    Redo,
    Capabilities,
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
        commands: Vec<String>,
        command_schema: serde_json::Value,
    },
    Error {
        code: String,
        message: String,
    },
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
            result: AgentResult::Error {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}
/// Desktop and headless sessions call this same function on their owning thread.
/// Mutation requests target both project identity and revision, so opening a
/// replacement project cannot retarget a queued or stale agent plan.
pub fn handle(editor: &mut Editor, request: AgentRequest) -> AgentResponse {
    let mut result = None;
    if matches!(
        request.operation,
        AgentOperation::Apply { .. } | AgentOperation::Undo | AgentOperation::Redo
    ) {
        if request.expected_revision.is_none() {
            result = Some(AgentResult::Error {
                code: "revision_required".into(),
                message: "Inspect the session and include expected_revision for every mutation"
                    .into(),
            });
        } else if request.expected_revision != Some(editor.revision()) {
            result = Some(AgentResult::Error {
                code: "stale_revision".into(),
                message: format!(
                    "Expected revision {:?}; current {}",
                    request.expected_revision,
                    editor.revision()
                ),
            });
        } else if request.expected_project_id.is_none() {
            result = Some(AgentResult::Error {
                code: "project_required".into(),
                message: "Inspect the session and include expected_project_id for every mutation"
                    .into(),
            });
        } else if request.expected_project_id != Some(editor.project().id) {
            result = Some(AgentResult::Error {
                code: "stale_project".into(),
                message: "The session now contains a different project; inspect before editing"
                    .into(),
            });
        } else if request.expected_context_id.is_none() {
            result = Some(AgentResult::Error {
                code: "context_required".into(),
                message: "Include expected_context_id from inspect for every mutation".into(),
            });
        } else if request.expected_context_id != Some(editor.instance_id()) {
            result = Some(AgentResult::Error {
                code: "stale_context".into(),
                message: "The project was reloaded; inspect the new editing context before editing"
                    .into(),
            });
        }
    }
    let result = result.unwrap_or_else(|| match request.operation {
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
            Err(error) => AgentResult::Error {
                code: "invalid_edit".into(),
                message: error.to_string(),
            },
        },
        AgentOperation::Undo => AgentResult::History {
            changed: editor.undo(),
        },
        AgentOperation::Redo => AgentResult::History {
            changed: editor.redo(),
        },
        AgentOperation::Capabilities => AgentResult::Capabilities {
            protocol_version: PROTOCOL_VERSION,
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
    });
    AgentResponse {
        request_id: request.request_id,
        revision: editor.revision(),
        result,
    }
}
