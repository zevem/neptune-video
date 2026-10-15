//! Projects: create, open, save, look at, change settings, close; and the
//! Aster windows a project can be edited in.
use super::{Guard, batch, clip_view, frame_rate, stamp};
use crate::mcp::{errors::ToolError, server::AsterServer, time::Stamp, workspace::Workspace};
use editor_core::{CanvasSettings, Command, Project};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// The most clips `project_inspect` lists in one answer.
const MOST_CLIPS: usize = 2000;
/// The largest whole project `project_inspect {scope: full}` returns.
const MOST_FULL_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateParams {
    /// Where to make the project file (.aster), inside the server's roots.
    /// Without a path the project is in memory until project_save.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// "16:9" (1920×1080, the default), "9:16" (1080×1920) or "1:1".
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Frames a second; 23.976, 29.97 and 59.94 are the broadcast rates.
    #[serde(default)]
    pub frame_rate: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenParams {
    /// The project file (.aster), inside the server's roots.
    pub path: String,
    /// Open the recovery copy beside the file instead: unsaved work a
    /// window or a server left after a crash.
    #[serde(default)]
    pub recover: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveParams {
    /// Save as a new file here (.aster). Without it the project is saved to
    /// its own file. A server's project is saved after every edit already;
    /// a window's is saved when this is called.
    #[serde(default)]
    pub destination: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InspectParams {
    /// `summary` (the default): settings, tracks, media, markers.
    /// `timeline`: every clip with its times and IDs. `media`: every media
    /// file with its metadata. `full`: the whole project as stored.
    #[serde(default)]
    pub scope: Option<String>,
    /// Only this track's clips.
    #[serde(default)]
    pub track_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SettingsParams {
    #[serde(default)]
    pub name: Option<String>,
    /// "16:9", "9:16" or "1:1": sets the width and height.
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub frame_rate: Option<f64>,
    /// RGBA, 0 to 255 each.
    #[serde(default)]
    pub background: Option<[u8; 4]>,
    #[serde(default)]
    pub audio_sample_rate: Option<u32>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseParams {
    /// Close a project holding work no file has: one never saved is lost,
    /// recovered work stays in its recovery copy.
    #[serde(default)]
    pub discard: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DesktopParams {
    /// `list`: the Aster windows that answer. `attach`: edit the project
    /// open in a window from now on. `open`: open the project's file (or
    /// `path`) in a new Aster window and edit it there.
    pub op: String,
    /// The window to attach to, from `list`; needed when several are open.
    #[serde(default)]
    pub instance_id: Option<Uuid>,
    /// For `open`: the project file; the open project's by default.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SettingsView {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    /// The exact rate as numerator / denominator.
    pub frame_rate_ratio: [u32; 2],
    pub background: [u8; 4],
    pub audio_sample_rate: u32,
}

impl SettingsView {
    fn of(settings: &CanvasSettings) -> Self {
        Self {
            width: settings.width,
            height: settings.height,
            frame_rate: settings.frame_rate.fps(),
            frame_rate_ratio: [
                settings.frame_rate.numerator,
                settings.frame_rate.denominator,
            ],
            background: settings.background,
            audio_sample_rate: settings.audio_sample_rate,
        }
    }
}

/// Where the project is and what state it is in.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Status {
    /// `none`, `local` (this server edits and saves the file after every
    /// edit) or `desktop` (edits go to an open Aster window).
    pub authority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    pub dirty: bool,
    pub saving: bool,
    pub conflicted: bool,
    /// The person is in the middle of a change in the window.
    pub editing: bool,
    /// The window is asking the person something.
    pub asking: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// The last edit is this agent's own, so history {op: undo} takes it.
    pub agent_can_undo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Stamp>,
    pub tracks: usize,
    pub clips: usize,
    pub media: usize,
    /// The folders this server reads and writes in.
    pub roots: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_save: Option<editor_agent::SaveOutcome>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
}

fn status_of(workspace: &mut Workspace) -> Result<Status, ToolError> {
    let roots = workspace.roots.list().to_vec();
    if workspace.binding().is_none() {
        return Ok(Status {
            authority: "none".into(),
            project_id: None,
            context_id: None,
            name: None,
            path: None,
            revision: None,
            dirty: false,
            saving: false,
            conflicted: false,
            editing: false,
            asking: false,
            can_undo: false,
            can_redo: false,
            agent_can_undo: false,
            settings: None,
            duration: None,
            tracks: 0,
            clips: 0,
            media: 0,
            roots,
            last_save: None,
            notices: workspace.take_notices(),
        });
    }
    let snapshot = workspace.snapshot()?;
    let (_, document) = workspace.document()?;
    let project = &snapshot.project;
    Ok(Status {
        authority: workspace.authority_name().into(),
        project_id: Some(project.id),
        context_id: Some(snapshot.context_id),
        name: Some(project.name.clone()),
        path: document.path,
        revision: Some(snapshot.revision),
        dirty: document.dirty,
        saving: document.saving,
        conflicted: document.conflicted,
        editing: document.editing,
        asking: document.asking,
        can_undo: snapshot.can_undo,
        can_redo: snapshot.can_redo,
        agent_can_undo: workspace.agent_can_undo(&snapshot),
        settings: Some(SettingsView::of(&project.settings)),
        duration: Some(stamp(project.duration(), project)),
        tracks: project.tracks.len(),
        clips: project.tracks.iter().map(|track| track.clips.len()).sum(),
        media: project.assets.len(),
        roots,
        last_save: document.last_save,
        notices: workspace.take_notices(),
    })
}

/// The status, for the `aster://project` resource too.
pub async fn status(server: &AsterServer) -> Result<Status, ToolError> {
    server.on(status_of).await
}

fn canvas(
    mut settings: CanvasSettings,
    aspect: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f64>,
) -> Result<CanvasSettings, ToolError> {
    if let Some(aspect) = aspect {
        if !["16:9", "9:16", "1:1"].contains(&aspect) {
            return Err(ToolError::invalid(
                "aspect is \"16:9\", \"9:16\" or \"1:1\"",
            ));
        }
        let preset = CanvasSettings::preset(aspect);
        settings.width = preset.width;
        settings.height = preset.height;
    }
    if let Some(width) = width {
        settings.width = width;
    }
    if let Some(height) = height {
        settings.height = height;
    }
    if let Some(fps) = fps {
        settings.frame_rate = frame_rate(fps)?;
    }
    if !settings.validate() {
        return Err(ToolError::invalid(
            "Those settings are not valid: sides are 16 to 16384 pixels, at most 134 million \
             pixels, 1 to 240 frames a second, 8 to 192 kHz",
        ));
    }
    Ok(settings)
}

fn project_file(server: &AsterServer, given: &str, overwrite: bool) -> Result<PathBuf, ToolError> {
    let path = server
        .roots()
        .destination(std::path::Path::new(given), overwrite)?;
    if path
        .extension()
        .is_none_or(|extension| extension != "aster")
    {
        return Err(ToolError::invalid("A project file ends in .aster"));
    }
    Ok(path)
}

#[tool_router(router = project_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Where the project is (this server, or an Aster window), its revision, \
                       file, settings, save state and size. Start here.",
        annotations(read_only_hint = true)
    )]
    async fn project_status(&self) -> Result<Json<Status>, ToolError> {
        status(self).await.map(Json)
    }

    #[tool(
        description = "Make a new project, in a new .aster file (saved after every edit) or in \
                       memory. Replaces the open project, which must hold no unsaved work.",
        annotations(destructive_hint = false)
    )]
    async fn project_create(
        &self,
        Parameters(params): Parameters<CreateParams>,
    ) -> Result<Json<Status>, ToolError> {
        let path = match &params.path {
            Some(given) => Some(project_file(self, given, false)?),
            None => None,
        };
        let mut project = Project::new(params.name.as_deref().unwrap_or("Untitled"));
        project.settings = canvas(
            project.settings,
            params.aspect.as_deref(),
            params.width,
            params.height,
            params.frame_rate,
        )?;
        self.on(move |workspace| {
            workspace.create(project, path)?;
            status_of(workspace)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Open a .aster project. If an Aster window has the same file open, edits \
                       go to the window. recover: true opens the crash recovery copy.",
        annotations(destructive_hint = false)
    )]
    async fn project_open(
        &self,
        Parameters(params): Parameters<OpenParams>,
    ) -> Result<Json<Status>, ToolError> {
        let path = self.roots().file(std::path::Path::new(&params.path))?;
        self.on(move |workspace| {
            workspace.open(&path, params.recover)?;
            status_of(workspace)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Save the project to its file, or as a new .aster file (destination). \
                       In a window this is the window's save; read project_status for \
                       last_save when it is done."
    )]
    async fn project_save(
        &self,
        Parameters(params): Parameters<SaveParams>,
    ) -> Result<Json<Status>, ToolError> {
        let destination = match &params.destination {
            Some(given) => Some(project_file(self, given, false)?),
            None => None,
        };
        self.on(move |workspace| {
            workspace.save(destination)?;
            status_of(workspace)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Read the project: summary (default), timeline (every clip with IDs and \
                       times), media (every file's metadata) or full (the stored project).",
        annotations(read_only_hint = true)
    )]
    async fn project_inspect(
        &self,
        Parameters(params): Parameters<InspectParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let roots = self.shared.roots.clone();
        let scope = params.scope.unwrap_or_else(|| "summary".into());
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let project = &snapshot.project;
        let mut out = serde_json::Map::new();
        out.insert("project_id".into(), serde_json::json!(project.id));
        out.insert("revision".into(), snapshot.revision.into());
        out.insert("scope".into(), scope.clone().into());
        match scope.as_str() {
            "summary" => {
                out.insert("name".into(), project.name.clone().into());
                out.insert(
                    "settings".into(),
                    serde_json::to_value(SettingsView::of(&project.settings)).unwrap_or_default(),
                );
                out.insert(
                    "duration".into(),
                    serde_json::to_value(stamp(project.duration(), project)).unwrap_or_default(),
                );
                out.insert(
                    "tracks".into(),
                    project
                        .tracks
                        .iter()
                        .enumerate()
                        .map(|(index, track)| {
                            serde_json::json!({
                                "track_id": track.id, "index": index, "name": track.name,
                                "kind": track.kind, "clips": track.clips.len(),
                                "locked": track.locked, "hidden": track.hidden, "muted": track.muted,
                                "solo": track.solo, "gain": track.gain, "language": track.language,
                                "end": stamp(track.clips.iter().map(|c| c.end()).max().unwrap_or_default(), project),
                            })
                        })
                        .collect(),
                );
                out.insert("media".into(), media_list(project, &roots).into());
                out.insert(
                    "markers".into(),
                    project
                        .markers
                        .iter()
                        .map(|marker| serde_json::json!({"marker_id": marker.id, "at": stamp(marker.time, project), "label": marker.label}))
                        .collect(),
                );
            }
            "timeline" => {
                let mut clips = Vec::new();
                let mut truncated = false;
                for track in &project.tracks {
                    if params.track_id.is_some_and(|id| id != track.id) {
                        continue;
                    }
                    for clip in &track.clips {
                        if clips.len() == MOST_CLIPS {
                            truncated = true;
                            break;
                        }
                        clips.push(clip_view(project, track, clip));
                    }
                }
                out.insert(
                    "tracks".into(),
                    project
                        .tracks
                        .iter()
                        .enumerate()
                        .map(|(index, track)| serde_json::json!({"track_id": track.id, "index": index, "name": track.name, "kind": track.kind, "locked": track.locked}))
                        .collect(),
                );
                out.insert("clips".into(), clips.into());
                out.insert("truncated".into(), truncated.into());
            }
            "media" => {
                out.insert("media".into(), media_list(project, &roots).into());
                out.insert(
                    "folders".into(),
                    project
                        .folders
                        .values()
                        .map(|folder| serde_json::json!({"folder_id": folder.id, "name": folder.name, "parent_id": folder.parent_id}))
                        .collect(),
                );
            }
            "full" => {
                let value = serde_json::to_value(&**project).unwrap_or_default();
                let size = serde_json::to_vec(&value).map_or(0, |bytes| bytes.len());
                if size > MOST_FULL_BYTES {
                    return Err(ToolError::new(
                        crate::mcp::errors::codes::TOO_LARGE,
                        format!(
                            "The project is {size} bytes; read scope timeline or media instead"
                        ),
                    ));
                }
                out.insert("project".into(), value);
            }
            other => {
                return Err(ToolError::invalid(format!(
                    "scope is summary, timeline, media or full, not {other}"
                )));
            }
        }
        Ok(Json(out))
    }

    #[tool(
        description = "Rename the project or change its canvas: size, aspect, frame rate, \
                          background, sample rate. One undoable edit."
    )]
    async fn project_settings(
        &self,
        Parameters(params): Parameters<SettingsParams>,
    ) -> Result<Json<crate::mcp::workspace::Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let mut commands = Vec::new();
            if let Some(name) = &params.name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(ToolError::invalid("A project name cannot be empty"));
                }
                commands.push(Command::RenameProject {
                    name: name.to_owned(),
                });
            }
            let mut settings = canvas(
                project.settings.clone(),
                params.aspect.as_deref(),
                params.width,
                params.height,
                params.frame_rate,
            )?;
            if let Some(background) = params.background {
                settings.background = background;
            }
            if let Some(rate) = params.audio_sample_rate {
                settings.audio_sample_rate = rate;
            }
            if settings != project.settings {
                commands.push(Command::SetSettings { settings });
            }
            batch("Project settings", commands)
        })
        .await
    }

    #[tool(
        description = "Let go of the project. A server's project is saved after every \
                          edit; a window's stays open in the window."
    )]
    async fn project_close(
        &self,
        Parameters(params): Parameters<CloseParams>,
    ) -> Result<Json<Status>, ToolError> {
        self.on(move |workspace| {
            workspace.close(params.discard)?;
            status_of(workspace)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Aster windows: list those running, attach to one so edits appear in \
                          it at once, or open the project in a new window."
    )]
    async fn desktop(
        &self,
        Parameters(params): Parameters<DesktopParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let path = match (&params.path, params.op.as_str()) {
            (Some(given), "open") => Some(self.roots().file(std::path::Path::new(given))?),
            _ => None,
        };
        let op = params.op.clone();
        let instance = params.instance_id;
        self.on(move |workspace| {
            let mut out = serde_json::Map::new();
            match op.as_str() {
                "list" => {
                    let windows: Vec<serde_json::Value> = workspace
                        .discover(true)
                        .into_iter()
                        .map(|session| {
                            serde_json::json!({
                                "instance_id": session.descriptor.instance_id,
                                "name": session.document.name,
                                "path": session.document.path,
                                "dirty": session.document.dirty,
                                "project_id": session.document.project_id,
                            })
                        })
                        .collect();
                    out.insert("windows".into(), windows.into());
                }
                "attach" | "open" => {
                    let session = if op == "attach" {
                        workspace.attach(instance)?
                    } else {
                        workspace.launch_desktop(path)?
                    };
                    out.insert(
                        "instance_id".into(),
                        serde_json::json!(session.descriptor.instance_id),
                    );
                    out.insert(
                        "status".into(),
                        serde_json::to_value(status_of(workspace)?).unwrap_or_default(),
                    );
                }
                other => {
                    return Err(ToolError::invalid(format!(
                        "op is list, attach or open, not {other}"
                    )));
                }
            }
            Ok(out)
        })
        .await
        .map(Json)
    }
}

/// Every media file with what the project knows of it, and whether this
/// server may read it.
fn media_list(project: &Project, roots: &crate::mcp::paths::Roots) -> Vec<serde_json::Value> {
    project
        .assets
        .values()
        .map(|asset| {
            serde_json::json!({
                "asset_id": asset.id,
                "name": asset.name,
                "kind": asset.kind,
                "path": asset.path,
                "duration": stamp(asset.duration, project),
                "width": asset.width,
                "height": asset.height,
                "frame_rate": asset.frame_rate.map(|rate| rate.fps()),
                "has_audio": asset.has_audio,
                "folder_id": asset.folder_id,
                "readable": roots.readable(&asset.path),
            })
        })
        .collect()
}
