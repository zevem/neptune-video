//! History: the undo and redo entries, undo and redo of the agent's own
//! work, and checkpoints beside the project file.
use crate::mcp::{
    errors::{ToolError, codes},
    server::AsterServer,
    workspace::Edited,
};
use editor_core::Command;
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// `list`, `undo` or `redo`.
    pub op: String,
    /// Undo or redo an entry that is not this agent's own (the person's,
    /// or another client's).
    #[serde(default)]
    pub allow_person: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckpointParams {
    /// `create` (a copy of the project as it is now, beside its file),
    /// `list`, or `restore` (one undoable edit back to a checkpoint).
    pub op: String,
    #[serde(default)]
    pub label: Option<String>,
    /// `restore`: the checkpoint's path from `list`; or give `label` for
    /// the newest with that label.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HistoryView {
    pub revision: u64,
    /// Undo entries, oldest first.
    pub past: Vec<String>,
    /// Redo entries, the next first.
    pub future: Vec<String>,
    /// Whether the last entry is this agent's own.
    pub agent_can_undo: bool,
}

pub async fn list(server: &AsterServer) -> Result<HistoryView, ToolError> {
    server
        .on(|workspace| {
            let snapshot = workspace.snapshot()?;
            let (past, future) = workspace.history()?;
            Ok(HistoryView {
                revision: snapshot.revision,
                past,
                future,
                agent_can_undo: workspace.agent_can_undo(&snapshot),
            })
        })
        .await
}

fn file_of(path: Option<PathBuf>) -> Result<PathBuf, ToolError> {
    path.ok_or_else(|| {
        ToolError::invalid(
            "Checkpoints are kept beside the project's file; save it first with project_save \
             {destination}",
        )
    })
}

#[tool_router(router = history_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "History: list the undo and redo entries, or undo / redo. Undo takes \
                          back only this agent's own last edit unless allow_person."
    )]
    async fn history(
        &self,
        Parameters(params): Parameters<HistoryParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let mut out = serde_json::Map::new();
        match params.op.as_str() {
            "list" => {
                let view = list(self).await?;
                out.insert(
                    "history".into(),
                    serde_json::to_value(view).unwrap_or_default(),
                );
            }
            "undo" | "redo" => {
                let redo = params.op == "redo";
                let allow = params.allow_person;
                let edited: Edited = self
                    .on(move |workspace| workspace.step(redo, allow))
                    .await?;
                out.insert(
                    "edit".into(),
                    serde_json::to_value(edited).unwrap_or_default(),
                );
            }
            other => {
                return Err(ToolError::invalid(format!(
                    "op is list, undo or redo, not {other}"
                )));
            }
        }
        Ok(Json(out))
    }

    #[tool(
        description = "Checkpoints of the project beside its file: create one, list them, or \
                          restore one as a single undoable edit."
    )]
    async fn checkpoint(
        &self,
        Parameters(params): Parameters<CheckpointParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let mut out = serde_json::Map::new();
        let (snapshot, path) = self
            .on(|workspace| {
                let snapshot = workspace.snapshot()?;
                Ok((
                    snapshot,
                    workspace.binding().and_then(|binding| binding.path.clone()),
                ))
            })
            .await?;
        match params.op.as_str() {
            "create" => {
                let file = file_of(path)?;
                let label = params
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("Revision {}", snapshot.revision));
                let project = snapshot.project.clone();
                let made = tokio::task::spawn_blocking(move || {
                    editor_store::checkpoint(&file, &project, &label)
                })
                .await
                .map_err(|_| ToolError::unavailable("The checkpoint was not written"))??;
                out.insert("path".into(), made.display().to_string().into());
                out.insert("revision".into(), snapshot.revision.into());
            }
            "list" => {
                let file = file_of(path)?;
                let found =
                    tokio::task::spawn_blocking(move || editor_store::list_checkpoints(&file))
                        .await
                        .map_err(|_| ToolError::unavailable("The checkpoints were not read"))??;
                out.insert(
                    "checkpoints".into(),
                    found
                        .iter()
                        .map(|checkpoint| serde_json::json!({"path": checkpoint.path, "label": checkpoint.label}))
                        .collect(),
                );
            }
            "restore" => {
                let file = file_of(path)?;
                let chosen = match (&params.path, &params.label) {
                    (Some(given), _) => {
                        let given = self.roots().file(std::path::Path::new(given))?;
                        let listed = tokio::task::spawn_blocking({
                            let file = file.clone();
                            move || editor_store::list_checkpoints(&file)
                        })
                        .await
                        .map_err(|_| ToolError::unavailable("The checkpoints were not read"))??;
                        if !listed
                            .iter()
                            .any(|c| std::fs::canonicalize(&c.path).is_ok_and(|p| p == given))
                        {
                            return Err(ToolError::not_found(
                                "That is not one of this project's checkpoints",
                            ));
                        }
                        given
                    }
                    (None, Some(label)) => {
                        let wanted = label.to_lowercase();
                        let listed = tokio::task::spawn_blocking(move || {
                            editor_store::list_checkpoints(&file)
                        })
                        .await
                        .map_err(|_| ToolError::unavailable("The checkpoints were not read"))??;
                        listed
                            .into_iter()
                            .find(|c| c.label.to_lowercase() == wanted)
                            .map(|c| c.path)
                            .ok_or_else(|| {
                                ToolError::not_found(format!("No checkpoint labelled {label}"))
                            })?
                    }
                    (None, None) => return Err(ToolError::invalid("Give path or label")),
                };
                let restored =
                    tokio::task::spawn_blocking(move || editor_store::load_project(&chosen))
                        .await
                        .map_err(|_| ToolError::unavailable("The checkpoint was not read"))??;
                if restored.id != snapshot.project.id {
                    return Err(ToolError::new(
                        codes::PROJECT_CHANGED,
                        "That checkpoint is of another project",
                    ));
                }
                let expected = params.expected_revision;
                let edited = self
                    .on(move |workspace| {
                        let snapshot = workspace.snapshot()?;
                        let command = Command::RestoreProject {
                            project: Box::new(restored),
                        };
                        // apply checks every path the checkpoint names.
                        workspace.apply(&snapshot, command, expected, false, true)
                    })
                    .await?;
                out.insert(
                    "edit".into(),
                    serde_json::to_value(edited).unwrap_or_default(),
                );
            }
            other => {
                return Err(ToolError::invalid(format!(
                    "op is create, list or restore, not {other}"
                )));
            }
        }
        Ok(Json(out))
    }
}
