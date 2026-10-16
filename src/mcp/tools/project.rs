//! Projects: create, open, save, look at, change settings, close; and the
//! Aster windows a project can be edited in.
use super::{Guard, batch, clip_view, frame_rate, stamp};
use crate::mcp::{
    errors::ToolError,
    server::AsterServer,
    time::{Stamp, TimeSpec},
    workspace::Workspace,
};
use editor_core::{CanvasSettings, Command, Project};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// The most clips `project_inspect` lists in one answer.
const MOST_CLIPS: usize = 500;
const DEFAULT_ITEMS: usize = 50;
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
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub wait_if_busy: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct InspectParams {
    /// summary (default), tracks (no media or clips), timeline, clips, media or full.
    #[serde(default)]
    pub scope: Option<String>,
    /// Legacy single-track filter; combines with track_ids.
    #[serde(default)]
    pub track_id: Option<Uuid>,
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
    #[serde(default)]
    pub clip_ids: Vec<Uuid>,
    #[serde(default)]
    pub asset_ids: Vec<Uuid>,
    /// Clips intersecting this half-open timeline interval; exact times, not snapped.
    #[serde(default)]
    pub time_range: Option<InspectRange>,
    /// full returns {track_id, clip}, where clip is the exact add_clip core object.
    #[serde(default)]
    pub detail: Option<String>,
    /// Only these top-level clip/media fields (IDs always included).
    #[serde(default)]
    pub fields: Vec<String>,
    /// Page size, 1 to 500; default 50. Use revision to detect changes between pages.
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InspectRange {
    pub start: TimeSpec,
    pub end: TimeSpec,
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
            workspace.with_wait(params.wait_if_busy, |workspace| {
                workspace.save(destination, params.expected_revision)?;
                status_of(workspace)
            })
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Read live project state: summary, tracks, timeline, clips, media or full. \
                       Filter and page clips/media; detail full returns exact add_clip objects.",
        annotations(read_only_hint = true)
    )]
    async fn project_inspect(
        &self,
        Parameters(params): Parameters<InspectParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        inspect(&snapshot, &self.shared.roots, params).map(Json)
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

/// Build bounded, filtered views from the authority's live immutable snapshot.
fn inspect(
    snapshot: &crate::mcp::workspace::Snapshot,
    roots: &crate::mcp::paths::Roots,
    params: InspectParams,
) -> Result<serde_json::Map<String, serde_json::Value>, ToolError> {
    use serde_json::json;
    let project = &snapshot.project;
    let scope = params.scope.as_deref().unwrap_or("summary");
    let limit = params.limit.unwrap_or(DEFAULT_ITEMS);
    if !(1..=MOST_CLIPS).contains(&limit) {
        return Err(ToolError::invalid(format!("limit is 1 to {MOST_CLIPS}")));
    }
    let full = match params.detail.as_deref() {
        None | Some("summary") => false,
        Some("full") => true,
        Some(_) => return Err(ToolError::invalid("detail is summary or full")),
    };
    let range = params
        .time_range
        .as_ref()
        .map(|range| {
            let start = range.start.exact(project.settings.frame_rate)?;
            let end = range.end.exact(project.settings.frame_rate)?;
            if start >= end {
                return Err(ToolError::invalid("time_range.end must follow start"));
            }
            Ok((start, end))
        })
        .transpose()?;
    for id in params.track_id.iter().chain(&params.track_ids) {
        super::track(project, *id)?;
    }
    for id in &params.clip_ids {
        super::clip(project, *id)?;
    }
    for id in &params.asset_ids {
        if !project.assets.contains_key(id) {
            return Err(ToolError::not_found(format!(
                "No asset {id} in the project"
            )));
        }
    }
    let tracks: Vec<_> = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| {
            params.track_id.is_none_or(|id| id == track.id)
                && (params.track_ids.is_empty() || params.track_ids.contains(&track.id))
        })
        .collect();
    let track_offset = if matches!(scope, "summary" | "tracks") {
        params.offset
    } else {
        0
    };
    let track_views: Vec<_> = tracks.iter().skip(track_offset).take(limit).map(|(index, track)| json!({
        "track_id": track.id, "index": index, "name": track.name, "kind": track.kind,
        "clips": track.clips.len(), "locked": track.locked, "hidden": track.hidden,
        "muted": track.muted, "solo": track.solo, "gain": track.gain,
        "language": track.language,
        "end": stamp(track.clips.iter().map(|clip| clip.end()).max().unwrap_or_default(), project),
    })).collect();
    let mut out = serde_json::Map::new();
    out.insert("project_id".into(), json!(project.id));
    out.insert("context_id".into(), json!(snapshot.context_id));
    out.insert("revision".into(), snapshot.revision.into());
    out.insert("scope".into(), scope.into());
    match scope {
        "summary" | "tracks" => {
            out.insert("tracks".into(), track_views.into());
            pagination(&mut out, tracks.len(), params.offset, limit);
            if scope == "summary" {
                out.insert("name".into(), project.name.clone().into());
                out.insert(
                    "settings".into(),
                    json!(SettingsView::of(&project.settings)),
                );
                out.insert("duration".into(), json!(stamp(project.duration(), project)));
                out.insert("media_count".into(), project.assets.len().into());
                out.insert(
                    "clips".into(),
                    project
                        .tracks
                        .iter()
                        .map(|track| track.clips.len())
                        .sum::<usize>()
                        .into(),
                );
                out.insert("markers".into(), project.markers.iter().skip(params.offset).take(limit).map(|marker| json!({
                    "marker_id": marker.id, "at": stamp(marker.time, project), "label": marker.label,
                })).collect());
                out.insert("marker_count".into(), project.markers.len().into());
                out.insert(
                    "hint".into(),
                    "Use scope media for paged asset metadata; tracks for only track state.".into(),
                );
            }
        }
        "timeline" | "clips" | "full"
            if scope != "full"
                || !params.clip_ids.is_empty()
                || !params.track_ids.is_empty()
                || params.track_id.is_some()
                || range.is_some()
                || !params.asset_ids.is_empty() =>
        {
            let mut total = 0usize;
            let mut clips = Vec::with_capacity(limit);
            let full = full || scope == "clips" || scope == "full";
            for (_, track) in &tracks {
                for clip in &track.clips {
                    if (!params.clip_ids.is_empty() && !params.clip_ids.contains(&clip.id))
                        || range
                            .is_some_and(|(start, end)| clip.start >= end || start >= clip.end())
                        || (!params.asset_ids.is_empty()
                            && clip
                                .asset_id()
                                .is_none_or(|id| !params.asset_ids.contains(&id)))
                    {
                        continue;
                    }
                    let on_page = total >= params.offset && clips.len() < limit;
                    total = total.saturating_add(1);
                    if !on_page {
                        continue;
                    }
                    let view = if full {
                        let mut value = serde_json::to_value(clip).unwrap_or_default();
                        project_fields(&mut value, &params.fields, &["id"]);
                        json!({"track_id": track.id, "clip": value})
                    } else {
                        let mut value = clip_view(project, track, clip);
                        project_fields(&mut value, &params.fields, &["clip_id", "track_id"]);
                        value
                    };
                    clips.push(view);
                }
            }
            out.insert("tracks".into(), track_views.into());
            out.insert("clips".into(), clips.into());
            out.insert("tracks_total".into(), tracks.len().into());
            out.insert("tracks_truncated".into(), (tracks.len() > limit).into());
            pagination(&mut out, total, params.offset, limit);
        }
        "media" => {
            let matching = || {
                project.assets.values().filter(|asset| {
                    params.asset_ids.is_empty() || params.asset_ids.contains(&asset.id)
                })
            };
            let total = matching().count();
            let page = matching()
                .skip(params.offset)
                .take(limit)
                .map(|asset| {
                    let mut value = media_view(project, roots, asset);
                    project_fields(&mut value, &params.fields, &["asset_id"]);
                    value
                })
                .collect::<Vec<_>>();
            out.insert("media".into(), page.into());
            pagination(&mut out, total, params.offset, limit);
            out.insert("folders".into(), project.folders.values().skip(params.offset).take(limit).map(|folder| json!({
                "folder_id": folder.id, "name": folder.name, "parent_id": folder.parent_id,
            })).collect());
            out.insert("folder_count".into(), project.folders.len().into());
        }
        "full" => {
            let value = serde_json::to_value(&**project).unwrap_or_default();
            let size = serde_json::to_vec(&value).map_or(0, |bytes| bytes.len());
            if size > MOST_FULL_BYTES {
                return Err(ToolError::new(
                    crate::mcp::errors::codes::TOO_LARGE,
                    format!(
                        "The project is {size} bytes; filter scope full by clip_ids or use paged timeline/media"
                    ),
                ));
            }
            out.insert("project".into(), value);
        }
        other => {
            return Err(ToolError::invalid(format!(
                "scope is summary, tracks, timeline, clips, media or full, not {other}"
            )));
        }
    }
    // Rich clips can be large individually; a bounded item count alone is insufficient.
    if serde_json::to_vec(&out).map_or(0, |bytes| bytes.len()) > MOST_FULL_BYTES {
        return Err(ToolError::new(
            crate::mcp::errors::codes::TOO_LARGE,
            "The inspection exceeds 4 MiB; use a smaller limit, clip_ids or fields",
        ));
    }
    Ok(out)
}

fn project_fields(value: &mut serde_json::Value, fields: &[String], identities: &[&str]) {
    if !fields.is_empty()
        && let Some(object) = value.as_object_mut()
    {
        object.retain(|key, _| identities.contains(&key.as_str()) || fields.contains(key));
    }
}

fn pagination(
    out: &mut serde_json::Map<String, serde_json::Value>,
    total: usize,
    offset: usize,
    limit: usize,
) {
    let next = offset.saturating_add(limit);
    out.insert("total".into(), total.into());
    out.insert("offset".into(), offset.into());
    out.insert("truncated".into(), (next < total).into());
    out.insert(
        "next_offset".into(),
        if next < total {
            next.into()
        } else {
            serde_json::Value::Null
        },
    );
}

/// Every media file with what the project knows of it, and whether this
/// server may read it.
fn media_view(
    project: &Project,
    roots: &crate::mcp::paths::Roots,
    asset: &editor_core::Asset,
) -> serde_json::Value {
    serde_json::json!({
        "asset_id": asset.id, "name": asset.name, "kind": asset.kind, "path": asset.path,
        "duration": stamp(asset.duration, project), "width": asset.width, "height": asset.height,
        "frame_rate": asset.frame_rate.map(|rate| rate.fps()), "has_audio": asset.has_audio,
        "folder_id": asset.folder_id, "readable": roots.readable(&asset.path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{paths::Roots, workspace::Snapshot};
    use editor_core::{Clip, Time, Track, TrackKind};
    use serde_json::json;
    use std::sync::Arc;

    fn fixture() -> (tempfile::TempDir, Roots, Snapshot) {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()], dir.path(), None).unwrap();
        let mut project = Project::new("Inspect");
        let mut track = Track::new("Captions", TrackKind::Text);
        for index in 0..75 {
            track.clips.push(Clip::text(
                format!("Caption {index}"),
                Time(index * 2_000_000),
                Time(2_000_000),
            ));
        }
        project.tracks = vec![Arc::new(track)];
        let snapshot = Snapshot {
            project: Arc::new(project),
            revision: 9,
            context_id: Uuid::new_v4(),
            can_undo: true,
            can_redo: false,
        };
        (dir, roots, snapshot)
    }

    fn params(value: serde_json::Value) -> InspectParams {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn inspection_pages_clips_and_keeps_summary_and_tracks_slim() {
        let (_dir, roots, snapshot) = fixture();
        let summary = inspect(&snapshot, &roots, InspectParams::default()).unwrap();
        assert!(!summary.contains_key("media"));
        assert_eq!(summary["clips"], 75);
        let tracks = inspect(&snapshot, &roots, params(json!({"scope":"tracks"}))).unwrap();
        assert!(!tracks.contains_key("clips"));
        assert_eq!(tracks["tracks"][0]["clips"], 75);
        let first = inspect(&snapshot, &roots, params(json!({"scope":"timeline"}))).unwrap();
        assert_eq!(first["clips"].as_array().unwrap().len(), 50);
        assert_eq!(first["total"], 75);
        assert_eq!(first["next_offset"], 50);
        let second = inspect(
            &snapshot,
            &roots,
            params(json!({"scope":"timeline", "offset":50})),
        )
        .unwrap();
        assert_eq!(second["clips"].as_array().unwrap().len(), 25);
        assert!(second["next_offset"].is_null());
        assert_ne!(first["clips"][0]["clip_id"], second["clips"][0]["clip_id"]);
    }

    #[test]
    fn full_selected_clips_round_trip_to_add_clip_without_saving() {
        let (_dir, roots, snapshot) = fixture();
        let clip = &snapshot.project.tracks[0].clips[6];
        let selected = inspect(
            &snapshot,
            &roots,
            params(json!({"scope":"full", "clip_ids":[clip.id]})),
        )
        .unwrap();
        let returned: Clip = serde_json::from_value(selected["clips"][0]["clip"].clone()).unwrap();
        assert_eq!(&returned, clip);
        let command: Command = serde_json::from_value(json!({
            "type":"add_clip", "track_id":snapshot.project.tracks[0].id,
            "clip":selected["clips"][0]["clip"],
        }))
        .unwrap();
        assert!(matches!(command, Command::AddClip { clip: returned, .. } if returned == *clip));
        assert_eq!(selected["revision"], 9);
    }

    #[test]
    fn inspection_combines_exact_range_ids_and_field_projection() {
        let (_dir, roots, snapshot) = fixture();
        let track = &snapshot.project.tracks[0];
        let clip = &track.clips[1];
        let selected = inspect(
            &snapshot,
            &roots,
            params(json!({
                "scope":"timeline", "track_ids":[track.id], "clip_ids":[clip.id],
                "time_range":{"start":{"us":2_000_001},"end":{"us":4_000_000}},
                "fields":["name"],
            })),
        )
        .unwrap();
        assert_eq!(
            selected["clips"],
            json!([{"clip_id":clip.id,"track_id":track.id,"name":clip.name}])
        );
        let touching = inspect(
            &snapshot,
            &roots,
            params(json!({
                "scope":"clips", "clip_ids":[clip.id],
                "time_range":{"start":{"us":4_000_000},"end":{"us":5_000_000}},
            })),
        )
        .unwrap();
        assert_eq!(touching["total"], 0);
        assert!(
            inspect(
                &snapshot,
                &roots,
                params(json!({"scope":"timeline", "track_ids":[Uuid::new_v4()]}))
            )
            .is_err()
        );
        assert!(inspect(&snapshot, &roots, params(json!({"limit":501}))).is_err());
    }
}
