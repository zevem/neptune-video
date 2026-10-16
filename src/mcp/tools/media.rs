//! Media: import files and folders, read what a file holds, remove media,
//! and sort it into folders.
use super::{Guard, blocking, refused, stamp};
use crate::mcp::{
    errors::{ToolError, codes},
    server::AsterServer,
    workspace::Edited,
};
use editor_core::{Command, MediaFolder};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// The most files and folders one import names.
const MOST_PATHS: usize = 256;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportParams {
    /// Files or folders inside the server's roots. A folder is walked, its
    /// sub-folders kept as media folders.
    pub paths: Vec<String>,
    /// Put the imported media in this folder.
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Imported {
    /// The edit that added the new media; none when all was there already.
    pub edit: Option<Edited>,
    /// Each media file now in the project, new or already there.
    pub media: Vec<serde_json::Value>,
    /// Files that could not be imported, and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InspectParams {
    /// Media UUID or unique hexadecimal prefix (at least four digits).
    #[serde(default)]
    pub asset_id: Option<String>,
    /// Or a file inside the roots, imported or not.
    #[serde(default)]
    pub path: Option<String>,
    /// Include every stream and format field ffprobe reports.
    #[serde(default)]
    pub details: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UsageParams {
    /// Media UUID or unique hexadecimal prefix (at least four digits).
    pub asset_id: String,
    #[serde(default)]
    pub offset: usize,
    /// At most 500 references; 50 by default.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveParams {
    pub asset_ids: Vec<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FolderParams {
    /// `create` a folder, or `move` media into one (or out of all with no
    /// folder_id).
    pub op: String,
    /// For `create`.
    #[serde(default)]
    pub name: Option<String>,
    /// For `create`: the folder to make it in.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// For `move`.
    #[serde(default)]
    pub asset_ids: Vec<Uuid>,
    /// For `move`: the folder to move into.
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

fn asset_view(
    project: &editor_core::Project,
    asset: &editor_core::Asset,
    new: bool,
) -> serde_json::Value {
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
        "file_size": asset.file_size,
        "folder_id": asset.folder_id,
        "new": new,
    })
}

#[tool_router(router = media_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Import media files or folders (video, audio, images, GIFs, fonts, \
                          .cube LUTs) into the project as one undoable edit. Files already in \
                          the project are not imported twice. Place them with clip_add."
    )]
    async fn media_import(
        &self,
        Parameters(params): Parameters<ImportParams>,
        ct: CancellationToken,
    ) -> Result<Json<Imported>, ToolError> {
        if params.paths.is_empty() || params.paths.len() > MOST_PATHS {
            return Err(ToolError::invalid(format!(
                "Name 1 to {MOST_PATHS} files or folders"
            )));
        }
        let mut paths = Vec::new();
        for given in &params.paths {
            paths.push(self.roots().entry(std::path::Path::new(given))?);
        }
        let roots = self.shared.roots.clone();
        let within = roots.clone();
        let probed = blocking(ct, move |cancel| {
            // A link in a folder that leads outside the roots is not read.
            let imported =
                editor_media::inspect::import_now(paths, cancel, &|path| within.contains(path));
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(ToolError::new("cancelled", "The import was cancelled"));
            }
            Ok(imported)
        })
        .await?;
        let mut errors: Vec<serde_json::Value> = (probed.errors.iter())
            .map(|error| serde_json::json!({"path": error.path, "message": error.message}))
            .collect();
        // A link in a folder may lead outside the roots: such media is not
        // taken.
        let (assets, outside): (Vec<_>, Vec<_>) =
            (probed.assets.into_iter()).partition(|asset| roots.readable(&asset.path));
        errors.extend(outside.iter().map(|asset| {
            serde_json::json!({"path": asset.path, "message": "It leads outside the folders this server may read"})
        }));
        let folders = probed.folders;
        let folder_id = params.folder_id;
        let guard = params.guard;
        let result = self
            .on(move |workspace| {
                workspace.with_wait(guard.wait_if_busy, |workspace| {
                    let snapshot = workspace.snapshot()?;
                    if let Some(expected) = guard.expected_revision
                        && expected != snapshot.revision
                    {
                        return Err(ToolError::new(
                            "stale_revision",
                            format!(
                                "The project is at revision {}, not {expected}",
                                snapshot.revision
                            ),
                        )
                        .at(snapshot.revision));
                    }
                    let project = &snapshot.project;
                    if let Some(folder) = folder_id
                        && !project.folders.contains_key(&folder)
                    {
                        return Err(ToolError::not_found(format!("No media folder {folder}")));
                    }
                    let known: BTreeMap<&std::path::Path, &editor_core::Asset> =
                        (project.assets.values())
                            .map(|asset| (asset.path.as_path(), asset))
                            .collect();
                    let mut fresh: Vec<editor_core::Asset> = Vec::new();
                    let mut media = Vec::new();
                    for mut asset in assets {
                        if let Some(existing) = known.get(asset.path.as_path()) {
                            media.push(asset_view(project, existing, false));
                            continue;
                        }
                        if asset.folder_id.is_none() {
                            asset.folder_id = folder_id;
                        }
                        fresh.push(asset);
                    }
                    // The folders of what is imported, and only those.
                    let used: std::collections::BTreeSet<Uuid> =
                        fresh.iter().filter_map(|a| a.folder_id).collect();
                    let mut keep: std::collections::BTreeSet<Uuid> = used.clone();
                    let by_id: BTreeMap<Uuid, &MediaFolder> =
                        folders.iter().map(|f| (f.id, f)).collect();
                    for id in &used {
                        let mut at = by_id.get(id).and_then(|folder| folder.parent_id);
                        while let Some(parent) = at {
                            if !keep.insert(parent) {
                                break;
                            }
                            at = by_id.get(&parent).and_then(|folder| folder.parent_id);
                        }
                    }
                    let mut commands: Vec<Command> = folders
                        .iter()
                        .filter(|folder| keep.contains(&folder.id))
                        .map(|folder| {
                            let mut folder = folder.clone();
                            if folder.parent_id.is_none() {
                                folder.parent_id = folder_id;
                            }
                            Command::AddFolder { folder }
                        })
                        .collect();
                    if fresh.is_empty() {
                        return Ok(Imported {
                            edit: None,
                            media,
                            errors,
                        });
                    }
                    for asset in &fresh {
                        media.push(asset_view(project, asset, true));
                    }
                    commands.push(Command::AddAssets { assets: fresh });
                    let command = Command::Batch {
                        label: "Import media".into(),
                        commands,
                    };
                    // Probed above, and only media inside the roots kept.
                    let edit = workspace.apply_imported(
                        &snapshot,
                        command,
                        guard.expected_revision,
                        guard.dry_run,
                    )?;
                    Ok(Imported {
                        edit: Some(edit),
                        media,
                        errors,
                    })
                })
            })
            .await?;
        if result.media.is_empty() {
            return Err(ToolError::new(codes::NOT_FOUND, "No media was found there")
                .with_detail(serde_json::json!({"errors": result.errors})));
        }
        Ok(Json(result))
    }

    #[tool(
        description = "What a media file holds: duration, size, rate, sound, and with details \
                       every stream ffprobe reports. Give asset_id, or a path to a file not \
                       yet imported.",
        annotations(read_only_hint = true)
    )]
    async fn media_inspect(
        &self,
        Parameters(params): Parameters<InspectParams>,
        ct: CancellationToken,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let (path, known) = match (params.asset_id, &params.path) {
            (Some(given), None) => {
                let snapshot = self.on(|workspace| workspace.snapshot()).await?;
                let id = super::resolve_asset_id(&snapshot.project, &given)?;
                let asset =
                    snapshot.project.assets.get(&id).ok_or_else(|| {
                        ToolError::not_found(format!("No media {id} in the project"))
                    })?;
                if !self.roots().readable(&asset.path) {
                    return Err(ToolError::new(
                        codes::OUTSIDE_ROOTS,
                        "That media is outside the folders this server may read, or missing",
                    ));
                }
                (
                    asset.path.clone(),
                    Some(asset_view(&snapshot.project, asset, false)),
                )
            }
            (None, Some(given)) => {
                let path = self.roots().file(std::path::Path::new(given))?;
                if !editor_media::inspect::importable(&path) {
                    return Err(ToolError::invalid(format!(
                        "{given} is not a kind of media Aster imports"
                    )));
                }
                (path, None)
            }
            _ => return Err(ToolError::invalid("Give asset_id or path")),
        };
        let details = params.details;
        let probed = blocking(ct, move |cancel| {
            let imported = editor_media::inspect::import_now(vec![path.clone()], cancel, &|_| true);
            // Every stream is read only of a file that probed as media.
            let details = if details && !imported.assets.is_empty() {
                Some(
                    editor_media::inspect::probe_details(&path)
                        .map_err(|error| ToolError::invalid(error.to_string()))?,
                )
            } else {
                None
            };
            Ok((imported, details))
        })
        .await?;
        let (imported, details) = probed;
        let mut out = serde_json::Map::new();
        if let Some(known) = known {
            out.insert("asset".into(), known);
        }
        match (imported.assets.first(), imported.errors.first()) {
            (Some(asset), _) => {
                let project = editor_core::Project::new("probe");
                let mut view = asset_view(&project, asset, false);
                if let Some(object) = view.as_object_mut() {
                    object.remove("new");
                    object.remove("asset_id");
                    object.remove("folder_id");
                }
                out.insert("probe".into(), view);
            }
            (None, Some(error)) => {
                out.insert("error".into(), error.message.clone().into());
            }
            (None, None) => {
                out.insert("error".into(), "Not a media file Aster reads".into());
            }
        }
        if let Some(details) = details {
            out.insert("details".into(), details);
        }
        Ok(Json(out))
    }

    #[tool(
        description = "Find clips and effects using an asset, without probing media or saving the project.",
        annotations(read_only_hint = true)
    )]
    async fn media_usage(
        &self,
        Parameters(params): Parameters<UsageParams>,
    ) -> Result<Json<serde_json::Value>, ToolError> {
        self.on(move |workspace| {
            let snapshot = workspace.snapshot()?;
            let project = &snapshot.project;
            let id = super::resolve_asset_id(project, &params.asset_id)?;
            let mut uses = Vec::new();
            let mut total = 0;
            let limit = params.limit.unwrap_or(50).clamp(1, 500);
            let mut keep = |item| {
                if total >= params.offset && uses.len() < limit { uses.push(item); }
                total += 1;
            };
            for track in &project.tracks {
                for clip in &track.clips {
                    if clip.asset_id() == Some(id) {
                        keep(serde_json::json!({"track_id": track.id, "clip_id": clip.id, "role": "source"}));
                    }
                    for effect in &clip.effects {
                        if effect.asset_id == Some(id) {
                            keep(serde_json::json!({"track_id": track.id, "clip_id": clip.id, "effect_id": effect.id, "role": "effect"}));
                        }
                    }
                }
            }
            let next = params.offset.saturating_add(uses.len());
            Ok(Json(serde_json::json!({"project_id": project.id, "revision": snapshot.revision, "asset_id": id, "uses": uses, "total": total, "next_offset": (next < total).then_some(next)})))
        }).await
    }

    #[tool(
        description = "Remove media from the project (not from disk). Refused while a clip \
                          or effect uses it; its trackers go with it."
    )]
    async fn media_remove(
        &self,
        Parameters(params): Parameters<RemoveParams>,
    ) -> Result<Json<Edited>, ToolError> {
        let ids = params.asset_ids;
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let mut commands = Vec::new();
            for id in &ids {
                if !project.assets.contains_key(id) {
                    return Err(ToolError::not_found(format!(
                        "No media {id} in the project"
                    )));
                }
                commands.push(crate::app::remove_asset_command(project, *id).0);
            }
            super::batch("Remove media", commands)
        })
        .await
    }

    #[tool(description = "Media folders: create one, or move media into a folder.")]
    async fn media_folder(
        &self,
        Parameters(params): Parameters<FolderParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            match params.op.as_str() {
                "create" => {
                    let name = params.name.as_deref().map(str::trim).unwrap_or_default();
                    if name.is_empty() {
                        return Err(ToolError::invalid("A folder needs a name"));
                    }
                    if let Some(parent) = params.parent_id
                        && !project.folders.contains_key(&parent)
                    {
                        return Err(ToolError::not_found(format!("No media folder {parent}")));
                    }
                    Ok(Command::AddFolder {
                        folder: MediaFolder {
                            id: Uuid::new_v4(),
                            name: name.to_owned(),
                            parent_id: params.parent_id,
                        },
                    })
                }
                "move" => {
                    if params.asset_ids.is_empty() {
                        return Err(ToolError::invalid("Name the media to move"));
                    }
                    let commands = params
                        .asset_ids
                        .iter()
                        .map(|id| Command::MoveAssetToFolder {
                            asset_id: *id,
                            folder_id: params.folder_id,
                        })
                        .collect();
                    super::batch("Move media", commands)
                }
                other => Err(refused(format!("op is create or move, not {other}"))),
            }
        })
        .await
    }
}
