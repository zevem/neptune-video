//! The MCP server: tools routed by namespace, the project and its history
//! as resources, and change notifications from the workspace.
use super::{
    errors::ToolError,
    frames::Frames,
    jobs::Jobs,
    paths::Roots,
    workspace::{Authority, Edited, Snapshot, Workspace},
};
use editor_core::Command;
use rmcp::{
    ErrorData, Json, Peer, RoleServer, ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{
        Implementation, ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, Resource, ResourceContents,
        ResourceUpdatedNotificationParam, ServerCapabilities, ServerConfig, SubscribeRequestParams,
        UnsubscribeRequestParams,
    },
    service::RequestContext,
    tool_handler,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PROJECT_URI: &str = "neptune-video://project";
pub const HISTORY_URI: &str = "neptune-video://history";
pub const JOBS_URI: &str = "neptune-video://jobs";

/// What every session of one server shares.
pub struct Shared {
    pub authority: Authority,
    pub jobs: Jobs,
    pub frames: Frames,
    pub roots: Arc<Roots>,
    pub models: Option<PathBuf>,
    pub presets: Option<PathBuf>,
    /// Peers subscribed to a resource, and what they subscribed to.
    pub watchers: Mutex<Vec<Watcher>>,
}

/// A subscribed session: its server instance, its peer and its resources.
#[derive(Clone)]
pub struct Watcher {
    session: uuid::Uuid,
    peer: Peer<RoleServer>,
    uris: HashSet<String>,
}

#[derive(Clone)]
pub struct NeptuneVideoServer {
    pub shared: Arc<Shared>,
    /// One per MCP session, to know its subscriptions.
    session: uuid::Uuid,
    tool_router: ToolRouter<Self>,
}

/// A client reads every tool's schema before its first call, so they are
/// kept small: results carry no output schema (the instructions describe
/// the edit result every editing tool returns, and each result is JSON in
/// `structuredContent`). Input schemas retain structure and prose for operation
/// choices, stored shapes, values and units. Shared targeting/guard guidance
/// lives in the server instructions and docs/mcp.md.
fn compact(mut router: ToolRouter<NeptuneVideoServer>) -> ToolRouter<NeptuneVideoServer> {
    fn quiet(value: &mut serde_json::Value, prose: bool) {
        match value {
            serde_json::Value::Object(object) => {
                object.remove("default");
                if !prose {
                    object.remove("description");
                }
                // Number widths say nothing a client checks; `uuid` stays.
                if object
                    .get("format")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|format| {
                        matches!(
                            format,
                            "int8"
                                | "int16"
                                | "int32"
                                | "int64"
                                | "uint"
                                | "uint8"
                                | "uint16"
                                | "uint32"
                                | "uint64"
                                | "float"
                                | "double"
                        )
                    })
                {
                    object.remove("format");
                }
                for (key, inner) in object.iter_mut() {
                    match (key.as_str(), inner) {
                        // Property and definition names are kept.
                        ("properties" | "$defs", serde_json::Value::Object(named)) => {
                            for (name, schema) in named.iter_mut() {
                                let semantic = matches!(
                                    name.as_str(),
                                    "op" | "changes"
                                        | "delta"
                                        | "detail"
                                        | "fields"
                                        | "params"
                                        | "settings"
                                        | "source_in"
                                        | "step"
                                        | "style"
                                        | "time_range"
                                        | "times"
                                        | "trim_animation"
                                        | "gain"
                                        | "db"
                                        | "mix"
                                        | "position"
                                        | "rotation"
                                        | "scale"
                                        | "threshold"
                                        | "amount"
                                        | "duration"
                                        | "key"
                                        | "rate"
                                        | "speed"
                                        | "strength"
                                        | "ease"
                                        | "target"
                                        | "value"
                                        | "size"
                                        | "format"
                                        | "cell_size"
                                );
                                quiet(schema, prose && semantic);
                            }
                        }
                        (_, inner) => quiet(inner, prose),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(|item| quiet(item, prose)),
            _ => {}
        }
    }
    for route in router.map.values_mut() {
        route.attr.output_schema = None;
        let mut value = serde_json::Value::Object((*route.attr.input_schema).clone());
        quiet(&mut value, true);
        if let serde_json::Value::Object(mut object) = value {
            object.remove("$schema");
            object.remove("title");
            route.attr.input_schema = Arc::new(object);
        }
    }
    router
}

pub const INSTRUCTIONS: &str = "\
Neptune Video is a desktop video editor. These tools edit one Neptune Video project (.nvp) with the same \
validated commands, history and file as the Neptune Video window.

Start with project_status. Make a project with project_create (a path makes the file at once), \
or open one with project_open; desktop {op: list|attach} edits a project open in a running Neptune Video \
window, where every edit appears at once. Import media with media_import, then place it with \
clip_add. Inspect scope tracks for slim track state; timeline/media are paged (50 by default, \
next_offset continues). Filter by track_ids, clip_ids, asset_ids or time_range {start,end}; fields \
selects top-level fields. scope clips with clip_ids returns exact live core clip objects, including \
animation/effects/content; no save or disk read is needed. Asset inputs accept a full UUID or a \
unique hexadecimal prefix of at least four digits; raw commands require full UUIDs.

Times: typed tools take a TimeSpec object with exactly one of {frame} (project frames), \
{seconds}, {us} (microseconds) or {timecode: \"HH:MM:SS:FF\"}, and report times in all of them. \
Timeline placement is frame-snapped; source_in and picture seconds/us are exact. Asset picture \
frame/timecode uses the source rate. \
edit_apply takes core commands, whose times are integer microseconds and whose keyframe times are \
relative to the clip's start.

Every edit is one undoable history entry. Editing tools return {project_id, context_id, revision, \
authority, label, changed, created: [IDs it added], dry_run?, notices?}. \
Pass expected_revision to refuse an edit when the project changed since you looked; dry_run: true \
checks an edit without applying it; allow_overlap: true lets clips overlap on a track, which the \
window never does. history {op: undo} takes back only your own last edit. \
When the project is in a window, a refusal with code person_editing or desktop_busy means the \
person is in the middle of something: wait_if_busy: true retries explicit refusals for five \
seconds (plus transport timeouts), keeping original project/context/revision guards. Indeterminate edits are \
never retried. history {op: changes_since,since_revision} includes human edits, undo and redo; \
if changes.complete is false, inspect again.

Use patch_clip for partial in-place edits with stable IDs: start,duration,source_in,asset_id and \
changes (stored objects merge, arrays replace, null clears optional fields). Shortening crops \
existing keys with an interpolated boundary; trim_animation: preserve validates strictly. \
clip_edit move/trim also preserves IDs; use it for source-aligned head trims. offset_clips shifts \
clip_ids by a signed delta. effect_edit and keyframes_set edit individual effects/curves.

Look at the result: frame_render draws exact times; contact_sheet accepts explicit times or evenly \
samples a range. only_tracks/exclude_tracks/solo_clip isolate picture layers without changing \
visibility. frames_export writes a long range as numbered pictures with bounded memory; output \
folders are created. loudness_start {scope: timeline,per_track: true} measures the stereo export \
mix after the master limiter; clip_ids measurements are for individual clip normalization. \
media_usage finds source/effect references to an asset. \
Long work runs as a job (transcribe_start, scenes_start, loudness_start, frames_export, render_export_start): \
poll it with job {op: wait}. Jobs never edit; apply their results with captions \
{op: from_transcript} or audio_edit {op: normalize}. edit_apply reaches every core command; \
edit_schema describes them. Files are read and written only inside the server's root folders.";

impl NeptuneVideoServer {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            session: uuid::Uuid::new_v4(),
            tool_router: compact(
                Self::project_tools()
                    + Self::media_tools()
                    + Self::timeline_tools()
                    + Self::effect_tools()
                    + Self::text_tools()
                    + Self::audio_tools()
                    + Self::analysis_tools()
                    + Self::render_tools()
                    + Self::history_tools()
                    + Self::job_tools()
                    + Self::edit_tools(),
            ),
        }
    }

    pub fn roots(&self) -> &Roots {
        &self.shared.roots
    }

    /// Runs `work` on the workspace.
    pub async fn on<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Workspace) -> Result<R, ToolError> + Send + 'static,
    ) -> Result<R, ToolError> {
        self.shared.authority.run(work).await
    }

    /// Builds one command from a fresh snapshot and applies it, guarded by
    /// the snapshot's revision (or `expected`).
    pub async fn edit(
        &self,
        guard: super::tools::Guard,
        build: impl FnOnce(&Snapshot) -> Result<Command, ToolError> + Send + 'static,
    ) -> Result<Json<Edited>, ToolError> {
        self.on(move |workspace| {
            workspace.with_wait(guard.wait_if_busy, |workspace| {
                let snapshot = workspace.snapshot()?;
                if let Some(expected) = guard.expected_revision
                    && expected != snapshot.revision
                {
                    return Err(ToolError::new(
                        "stale_revision",
                        format!(
                            "The project is at revision {}, not {expected}; look again before \
                         editing",
                            snapshot.revision
                        ),
                    )
                    .at(snapshot.revision));
                }
                let command = build(&snapshot)?;
                workspace.apply(
                    &snapshot,
                    command,
                    guard.expected_revision,
                    guard.dry_run,
                    guard.allow_overlap,
                )
            })
        })
        .await
        .map(Json)
    }

    async fn resource(&self, uri: &str) -> Result<serde_json::Value, ToolError> {
        match uri {
            PROJECT_URI => {
                let status = super::tools::project::status(self).await?;
                Ok(serde_json::to_value(status).unwrap_or_default())
            }
            HISTORY_URI => {
                let history = super::tools::history::list(self).await?;
                Ok(serde_json::to_value(history).unwrap_or_default())
            }
            JOBS_URI => {
                Ok(serde_json::to_value(super::tools::jobs::all(self).await).unwrap_or_default())
            }
            _ => Err(ToolError::not_found(format!("No resource {uri}"))),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for NeptuneVideoServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .build(),
        )
        .with_server_info(
            Implementation::new("neptune-video", env!("CARGO_PKG_VERSION"))
                .with_title("Neptune Video editor"),
        )
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resource = |uri: &str, name: &str, description: &str| {
            Resource::new(uri, name)
                .with_description(description)
                .with_mime_type("application/json")
        };
        Ok(ListResourcesResult {
            resources: vec![
                resource(
                    PROJECT_URI,
                    "project",
                    "The open project's status: authority, revision, file, settings, tracks",
                ),
                resource(HISTORY_URI, "history", "Undo and redo entries"),
                resource(
                    JOBS_URI,
                    "jobs",
                    "Long work and exports with their progress",
                ),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let value = self
            .resource(&request.uri)
            .await
            .map_err(|error| ErrorData::resource_not_found(error.to_string(), None))?;
        let text = serde_json::to_string_pretty(&value).unwrap_or_default();
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri).with_mime_type("application/json"),
        ])
        .into())
    }

    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if ![PROJECT_URI, HISTORY_URI, JOBS_URI].contains(&request.uri.as_str()) {
            return Err(ErrorData::resource_not_found(
                format!("No resource {}", request.uri),
                None,
            ));
        }
        let mut watchers = self
            .shared
            .watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        watchers.retain(|watcher| !watcher.peer.is_transport_closed());
        match watchers
            .iter_mut()
            .find(|watcher| watcher.session == self.session)
        {
            Some(watcher) => {
                watcher.uris.insert(request.uri);
            }
            None => watchers.push(Watcher {
                session: self.session,
                peer: context.peer.clone(),
                uris: HashSet::from([request.uri]),
            }),
        }
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let mut watchers = self
            .shared
            .watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for watcher in watchers.iter_mut() {
            if watcher.session == self.session {
                watcher.uris.remove(&request.uri);
            }
        }
        watchers.retain(|watcher| !watcher.uris.is_empty() && !watcher.peer.is_transport_closed());
        Ok(())
    }
}

/// Watches the project for changes made anywhere (the window, another
/// client) while someone is subscribed, and tells the subscribers. The
/// window answers `document` without waking, so watching costs it nothing.
pub async fn watch(shared: Arc<Shared>) {
    let mut last: Option<serde_json::Value> = None;
    let mut last_jobs = shared.jobs.generation();
    loop {
        // At most twice a second, however fast jobs report progress.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let watchers: Vec<Watcher> = {
            let mut watchers = shared
                .watchers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            watchers.retain(|watcher| !watcher.peer.is_transport_closed());
            watchers.clone()
        };
        if watchers.is_empty() {
            last = None;
            continue;
        }
        let key = shared
            .authority
            .run(|workspace| {
                let (revision, document) = workspace.document()?;
                Ok(serde_json::json!({
                    "context": document.context_id,
                    "project": document.project_id,
                    "path": document.path,
                    "revision": revision,
                    "version": document.file_version,
                    "dirty": document.dirty,
                    "saving": document.saving,
                    "conflicted": document.conflicted,
                    "authority": workspace.authority_name(),
                }))
            })
            .await
            .unwrap_or(serde_json::Value::Null);
        let jobs = shared.jobs.generation();
        let project_changed = last.as_ref() != Some(&key);
        let jobs_changed = jobs != last_jobs;
        last = Some(key);
        last_jobs = jobs;
        for Watcher { peer, uris, .. } in watchers {
            for uri in uris {
                let tell = match uri.as_str() {
                    PROJECT_URI | HISTORY_URI => project_changed,
                    JOBS_URI => jobs_changed,
                    _ => false,
                };
                if tell {
                    let _ = peer
                        .notify_resource_updated(ResourceUpdatedNotificationParam::new(uri))
                        .await;
                }
            }
        }
    }
}
