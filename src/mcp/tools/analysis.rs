//! Looking and listening: frames and contact sheets of the timeline or of
//! one media file, the cuts between shots, loudness, and speech heard with
//! word times. Pictures come back inline; long work runs as jobs whose
//! results the agent applies with ordinary tools.
use super::{check_readable, clip, stamp};
use crate::mcp::{
    errors::{ToolError, codes},
    frames::{self, Drawn, MOST_FRAMES},
    jobs::{JobInfo, Progress},
    server::AsterServer,
    time::TimeSpec,
};
use editor_core::{Project, Time};
use editor_media::{
    Loudness, LoudnessJob, SoundService,
    speech::{DETECTOR, MODELS, Model, SpeechEvent, SpeechJob, SpeechService, Transcription},
};
use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FrameParams {
    /// The times to draw, timeline times (or media times with asset_id).
    pub times: Vec<TimeSpec>,
    /// Draw a media file's own frames instead of the timeline.
    #[serde(default)]
    pub asset_id: Option<Uuid>,
    /// The longest side in pixels, 1280 by default, at most 2048.
    #[serde(default)]
    pub size: Option<u32>,
    /// `png` (the default) or `jpeg`.
    #[serde(default)]
    pub format: Option<String>,
    /// Also write the pictures to this file inside the roots (a number is
    /// added before the extension when there are several).
    #[serde(default)]
    pub save_to: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
    /// Leave the pictures out of the answer (with save_to).
    #[serde(default)]
    pub no_inline: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SheetParams {
    /// From here (the start by default)…
    #[serde(default)]
    pub from: Option<TimeSpec>,
    /// …to here (the end by default).
    #[serde(default)]
    pub to: Option<TimeSpec>,
    /// How many frames, evenly spaced: 12 by default, at most 48.
    #[serde(default)]
    pub count: Option<usize>,
    /// Frames across: 4 by default.
    #[serde(default)]
    pub columns: Option<u32>,
    /// Each frame's longest side, 320 by default.
    #[serde(default)]
    pub cell_size: Option<u32>,
    /// A media file's frames instead of the timeline.
    #[serde(default)]
    pub asset_id: Option<Uuid>,
    #[serde(default)]
    pub save_to: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScenesParams {
    /// A video in the project.
    #[serde(default)]
    pub asset_id: Option<Uuid>,
    /// Or a clip: its media between its in and out, with timeline times.
    #[serde(default)]
    pub clip_id: Option<Uuid>,
    /// 0 to 1; 0.35 by default. Lower finds softer cuts.
    #[serde(default)]
    pub threshold: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoudnessParams {
    /// Clips whose sound to measure, with their audio effects.
    pub clip_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ModelParams {
    /// `list` or `install` (download a model and the speech detector, a
    /// job).
    pub op: String,
    /// `install`: tiny, base, small, medium or large-v3-turbo.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TranscribeParams {
    /// An installed model: base by default.
    #[serde(default)]
    pub model: Option<String>,
    /// A language tag, or none to tell it from the sound.
    #[serde(default)]
    pub language: Option<String>,
    /// Write English whatever is spoken.
    #[serde(default)]
    pub translate: bool,
    /// The stretch of timeline to hear: all of it by default.
    #[serde(default)]
    pub from: Option<TimeSpec>,
    #[serde(default)]
    pub to: Option<TimeSpec>,
}

/// A tool answer with pictures in it.
pub fn with_pictures(value: serde_json::Value, pictures: Vec<(Vec<u8>, &str)>) -> CallToolResult {
    let mut result = CallToolResult::structured(value);
    for (bytes, mime) in pictures {
        result.content.push(ContentBlock::image(
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            mime,
        ));
    }
    result
}

/// What is drawn: the timeline, or one media file alone.
fn subject(project: &Arc<Project>, asset: Option<Uuid>) -> Result<Arc<Project>, ToolError> {
    match asset {
        None => Ok(project.clone()),
        Some(id) => {
            let asset = project
                .assets
                .get(&id)
                .ok_or_else(|| ToolError::not_found(format!("No media {id} in the project")))?;
            let mut preview = Project::asset_preview(asset)
                .map_err(|_| ToolError::invalid("That media has no picture"))?;
            preview.settings.width = asset.width.max(16);
            preview.settings.height = asset.height.max(16);
            if asset.kind == editor_core::MediaKind::Audio {
                return Err(ToolError::invalid("Sound has no picture"));
            }
            Ok(Arc::new(preview))
        }
    }
}

/// Writes a picture to a place `Roots::destination` gave.
fn write_picture(path: &std::path::Path, bytes: &[u8]) -> Result<(), ToolError> {
    crate::mcp::paths::write_new(path, bytes)
        .map_err(|error| ToolError::new(codes::STORAGE, format!("{}: {error}", path.display())))
}

fn numbered(path: &std::path::Path, index: usize, count: usize) -> PathBuf {
    if count == 1 {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{stem}-{:02}.{extension}", index + 1))
}

/// One speech job at a time in this server, as in the window: two model
/// downloads would write the same partial file, and two transcriptions
/// would share the processor for no gain.
static SPEECH_LANE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs one speech job on a service of its own until it answers, after any
/// other speech job of this server.
fn speech(job: SpeechJob, progress: &Progress) -> Result<SpeechEvent, ToolError> {
    let _lane = loop {
        match SPEECH_LANE.try_lock() {
            Ok(lane) => break lane,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => break poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if progress.cancelled() {
            return Err(ToolError::new("cancelled", "Cancelled"));
        }
        progress.set(0.0, "Waiting for another speech job");
        std::thread::sleep(Duration::from_millis(200));
    };
    let (wake, woken) = std::sync::mpsc::channel::<()>();
    let service = SpeechService::new(move || {
        let _ = wake.send(());
    })
    .map_err(|error| ToolError::unavailable(error.to_string()))?;
    let request = service
        .start(job)
        .map_err(|error| ToolError::unavailable(error.to_string()))?;
    let mut asked_to_stop: Option<std::time::Instant> = None;
    loop {
        if progress.cancelled() && asked_to_stop.is_none() {
            service.cancel(request);
            asked_to_stop = Some(std::time::Instant::now());
        }
        // A service that does not answer a cancel is not waited for.
        if asked_to_stop.is_some_and(|since| since.elapsed() > Duration::from_secs(10)) {
            return Err(ToolError::new("cancelled", "Cancelled"));
        }
        while let Some(event) = service.poll() {
            if event.request() == request {
                return Ok(event);
            }
        }
        if let Some(state) = service.progress() {
            progress.set(
                state.done,
                crate::speech::stage_text(Some(state.stage), asked_to_stop.is_some()),
            );
        }
        let _ = woken.recv_timeout(Duration::from_millis(250));
    }
}

fn model_view(model: &Model, dir: Option<&std::path::Path>) -> serde_json::Value {
    serde_json::json!({
        "model": model.id,
        "name": model.name,
        "note": model.note,
        "megabytes": model.megabytes(),
        "installed": dir.is_some_and(|dir| model.installed(dir)),
    })
}

#[tool_router(router = analysis_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Draw the timeline (or one media file) at given times as pictures, with \
                       effects, titles, captions and transitions as an export draws them. Use \
                       it to look at your edit.",
        annotations(read_only_hint = true)
    )]
    async fn frame_render(
        &self,
        Parameters(params): Parameters<FrameParams>,
    ) -> Result<CallToolResult, ToolError> {
        if params.times.is_empty() || params.times.len() > MOST_FRAMES {
            return Err(ToolError::invalid(format!(
                "Ask for 1 to {MOST_FRAMES} times"
            )));
        }
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let drawn_of = subject(&snapshot.project, params.asset_id)?;
        check_readable(&drawn_of, self.roots())?;
        let rate = snapshot.project.settings.frame_rate;
        let times = params
            .times
            .iter()
            .map(|time| time.at(rate))
            .collect::<Result<Vec<Time>, _>>()?;
        // Every file is checked before anything is drawn: one per time,
        // numbered when there are several.
        let save: Vec<PathBuf> = match &params.save_to {
            Some(given) => (0..times.len())
                .map(|index| {
                    self.roots().destination(
                        &numbered(std::path::Path::new(given), index, times.len()),
                        params.overwrite,
                    )
                })
                .collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        let jpeg = match params.format.as_deref() {
            None | Some("png") => false,
            Some("jpeg" | "jpg") => true,
            Some(other) => {
                return Err(ToolError::invalid(format!(
                    "format is png or jpeg, not {other}"
                )));
            }
        };
        let size = params.size.unwrap_or(1280);
        let drawn = self
            .shared
            .frames
            .draw(drawn_of.clone(), times, size)
            .await?;
        // Encoding and writing take a while; not on the protocol's threads.
        let (drawn, encoded, written) = tokio::task::spawn_blocking(move || {
            let encoded: Vec<Vec<u8>> = drawn
                .iter()
                .map(|picture| {
                    if jpeg {
                        frames::jpeg(picture.width, picture.height, &picture.rgba)
                    } else {
                        frames::png(picture)
                    }
                })
                .collect::<Result<_, _>>()?;
            let mut written = Vec::new();
            for (path, bytes) in save.iter().zip(&encoded) {
                write_picture(path, bytes)?;
                written.push(path.display().to_string());
            }
            Ok::<_, ToolError>((drawn, encoded, written))
        })
        .await
        .map_err(|_| ToolError::unavailable("Encoding the pictures failed"))??;
        let value = serde_json::json!({
            "revision": snapshot.revision,
            "frames": drawn.iter().map(|picture| serde_json::json!({
                "at": stamp(picture.at, &snapshot.project),
                "width": picture.width,
                "height": picture.height,
            })).collect::<Vec<_>>(),
            "saved": written,
        });
        let mime = if jpeg { "image/jpeg" } else { "image/png" };
        let pictures = if params.no_inline {
            Vec::new()
        } else {
            encoded.into_iter().map(|bytes| (bytes, mime)).collect()
        };
        Ok(with_pictures(value, pictures))
    }

    #[tool(
        description = "One picture of many frames, evenly spaced over the timeline (or a media \
                       file, or a stretch of either): an overview of the whole edit.",
        annotations(read_only_hint = true)
    )]
    async fn contact_sheet(
        &self,
        Parameters(params): Parameters<SheetParams>,
    ) -> Result<CallToolResult, ToolError> {
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let drawn_of = subject(&snapshot.project, params.asset_id)?;
        check_readable(&drawn_of, self.roots())?;
        let rate = snapshot.project.settings.frame_rate;
        let end = drawn_of.duration();
        if end <= Time::ZERO {
            return Err(ToolError::invalid(
                "There is nothing on the timeline to draw",
            ));
        }
        let from = params
            .from
            .as_ref()
            .map(|t| t.at(rate))
            .transpose()?
            .unwrap_or(Time::ZERO);
        let to = params
            .to
            .as_ref()
            .map(|t| t.at(rate))
            .transpose()?
            .unwrap_or(end)
            .min(end);
        if from >= to {
            return Err(ToolError::invalid("from must come before to"));
        }
        let count = params.count.unwrap_or(12).clamp(1, MOST_FRAMES);
        let span = to.0 - from.0;
        // The middle of each of `count` equal stretches.
        let times: Vec<Time> = (0..count)
            .map(|index| {
                let middle = from.0 + (span * (2 * index as i64 + 1)) / (2 * count as i64);
                crate::rules::quantize(Time(middle), rate)
            })
            .collect();
        let save = match &params.save_to {
            Some(given) => Some(
                self.roots()
                    .destination(std::path::Path::new(given), params.overwrite)?,
            ),
            None => None,
        };
        let columns = params.columns.unwrap_or(4).clamp(1, count as u32);
        // Cells small enough that the whole sheet fits the longest side a
        // picture handed over may have.
        let cell = params
            .cell_size
            .unwrap_or(320)
            .clamp(64, 640)
            .min(frames::sheet_cell(columns, count as u32));
        let drawn: Vec<Drawn> = self
            .shared
            .frames
            .draw(drawn_of.clone(), times, cell)
            .await?;
        let saving = save.clone();
        let (drawn, width, height, bytes) = tokio::task::spawn_blocking(move || {
            let save = saving;
            let (width, height, rgba) = frames::sheet(&drawn, columns);
            let bytes = frames::jpeg(width, height, &rgba)?;
            if let Some(path) = &save {
                write_picture(path, &bytes)?;
            }
            Ok::<_, ToolError>((drawn, width, height, bytes))
        })
        .await
        .map_err(|_| ToolError::unavailable("Encoding the sheet failed"))??;
        let value = serde_json::json!({
            "revision": snapshot.revision,
            "width": width,
            "height": height,
            "columns": columns,
            "cells": drawn.iter().enumerate().map(|(index, picture)| serde_json::json!({
                "index": index,
                "at": stamp(picture.at, &snapshot.project),
            })).collect::<Vec<_>>(),
            "saved": save.map(|path| path.display().to_string()),
        });
        Ok(with_pictures(value, vec![(bytes, "image/jpeg")]))
    }

    #[tool(
        description = "Find the cuts between shots in a video (a job): each with its media \
                          time, its timeline time for a clip, and how strong it is. Poll with \
                          job {op: wait}."
    )]
    async fn scenes_start(
        &self,
        Parameters(params): Parameters<ScenesParams>,
    ) -> Result<Json<JobInfo>, ToolError> {
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let project = snapshot.project.clone();
        let (asset, clip_place) = match (params.asset_id, params.clip_id) {
            (Some(id), None) => (
                project
                    .assets
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| ToolError::not_found(format!("No media {id}")))?,
                None,
            ),
            (None, Some(id)) => {
                let (_, found) = clip(&project, id)?;
                let asset_id = found
                    .asset_id()
                    .ok_or_else(|| ToolError::invalid("That clip is not media"))?;
                let asset = project
                    .assets
                    .get(&asset_id)
                    .cloned()
                    .ok_or_else(|| ToolError::not_found("Its media is gone"))?;
                (asset, Some(found.clone()))
            }
            _ => return Err(ToolError::invalid("Give asset_id or clip_id")),
        };
        if !self.roots().readable(&asset.path) {
            return Err(ToolError::new(
                codes::OUTSIDE_ROOTS,
                "That media is outside the folders this server may read",
            ));
        }
        let threshold = params.threshold.unwrap_or(0.35);
        let rate = project.settings.frame_rate;
        self.shared
            .jobs
            .start("scenes", "Looking for cuts", move |progress| {
                let (from, to) = match &clip_place {
                    Some(clip) => {
                        let (a, b) = clip.source_span();
                        (a.min(b), a.max(b))
                    }
                    None => (Time::ZERO, asset.duration),
                };
                let cuts = editor_media::inspect::scene_cuts(
                    &asset,
                    from,
                    to,
                    threshold,
                    progress.flag(),
                    &|done| progress.set(done, "Looking for cuts"),
                )
                .map_err(|error| ToolError::new(codes::JOB_FAILED, error.to_string()))?;
                let cuts: Vec<serde_json::Value> = cuts
                    .iter()
                    .map(|cut| {
                        let mut view = serde_json::json!({
                            "media_time": crate::mcp::time::Stamp::new(cut.at, rate),
                            "score": cut.score,
                        });
                        if let Some(clip) = &clip_place
                            && clip.retime.is_none()
                        {
                            let local = (cut.at.0 - clip.source_in.0) as f64 / clip.playback_rate;
                            let at = Time(clip.start.0 + local.round() as i64);
                            view["timeline_time"] =
                                serde_json::to_value(crate::mcp::time::Stamp::new(at, rate))
                                    .unwrap_or_default();
                        }
                        view
                    })
                    .collect();
                Ok(serde_json::json!({
                    "asset_id": asset.id,
                    "clip_id": clip_place.as_ref().map(|clip| clip.id),
                    "threshold": threshold,
                    "cuts": cuts,
                }))
            })
            .map(Json)
    }

    #[tool(
        description = "Measure clips' loudness (integrated LUFS and true peak) as a job, with \
                          their audio effects. Apply it with audio_edit {op: normalize}."
    )]
    async fn loudness_start(
        &self,
        Parameters(params): Parameters<LoudnessParams>,
    ) -> Result<Json<JobInfo>, ToolError> {
        if params.clip_ids.is_empty() || params.clip_ids.len() > crate::normalize::MOST_WAITING {
            return Err(ToolError::invalid(format!(
                "Name 1 to {} clips",
                crate::normalize::MOST_WAITING
            )));
        }
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let project = snapshot.project.clone();
        let project_id = project.id;
        let mut jobs = Vec::new();
        for id in &params.clip_ids {
            if let Some(reason) = crate::normalize::refusal(&project, *id) {
                return Err(ToolError::invalid(reason));
            }
            let (_, measured) = clip(&project, *id)?;
            let asset = measured
                .asset_id()
                .and_then(|asset| project.assets.get(&asset))
                .ok_or_else(|| ToolError::invalid("That clip has no media"))?;
            if !self.roots().readable(&asset.path) {
                return Err(ToolError::new(
                    codes::OUTSIDE_ROOTS,
                    "That media is outside the folders this server may read",
                ));
            }
            let (a, b) = measured.source_span();
            let key = crate::normalize::sound_key(&project, *id);
            jobs.push((
                (*id, key),
                LoudnessJob {
                    project: project.id,
                    asset: asset.clone(),
                    from: a.min(b),
                    span: Time(a.max(b).0 - a.min(b).0),
                    effects: measured
                        .effects
                        .iter()
                        .filter(|effect| effect.enabled && effect.kind.is_audio())
                        .cloned()
                        .collect(),
                },
            ));
        }
        self.shared.jobs.start("loudness", "Measuring", move |progress| {
            let (wake, woken) = std::sync::mpsc::channel::<()>();
            let service = SoundService::new(move || {
                let _ = wake.send(());
            })
            .map_err(|error| ToolError::unavailable(error.to_string()))?;
            let total = jobs.len();
            let mut clips = Vec::new();
            for (index, ((clip_id, key), job)) in jobs.into_iter().enumerate() {
                let request = service
                    .loudness(job)
                    .map_err(|error| ToolError::new(codes::JOB_FAILED, error.to_string()))?;
                let answer = loop {
                    if progress.cancelled() {
                        service.cancel_all();
                        return Err(ToolError::new("cancelled", "Cancelled"));
                    }
                    let mut found = None;
                    while let Some(event) = service.poll() {
                        if let editor_media::SoundEvent::Measured { request: r, result, .. } = event
                            && r == request
                        {
                            found = Some(result);
                        }
                    }
                    if let Some(found) = found {
                        break found;
                    }
                    let _ = woken.recv_timeout(Duration::from_millis(250));
                };
                clips.push(match answer {
                    Ok(Loudness::Measured { integrated, true_peak }) => serde_json::json!({
                        "clip_id": clip_id, "integrated_lufs": integrated, "true_peak_dbfs": true_peak,
                        "sound_key": key.map(|key| key.to_string()),
                        "gain_to_target": crate::normalize::volume(integrated, true_peak).map(|v| v.gain),
                    }),
                    Ok(Loudness::Silent) => serde_json::json!({"clip_id": clip_id, "status": crate::normalize::SILENT}),
                    Ok(Loudness::TooShort) => serde_json::json!({"clip_id": clip_id, "status": crate::normalize::TOO_SHORT}),
                    Err(message) => serde_json::json!({"clip_id": clip_id, "status": message}),
                });
                progress.set((index + 1) as f32 / total as f32, format!("Measured {} of {total}", index + 1));
            }
            Ok(serde_json::json!({
                "project_id": project_id,
                "target_lufs": crate::normalize::TARGET,
                "clips": clips,
            }))
        })
        .map(Json)
    }

    #[tool(
        description = "Speech models: list which are installed, or install one (a download \
                          job, checked against its published sum)."
    )]
    async fn transcription_models(
        &self,
        Parameters(params): Parameters<ModelParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let dir = self.shared.models.clone();
        let mut out = serde_json::Map::new();
        match params.op.as_str() {
            "list" => {
                out.insert(
                    "folder".into(),
                    dir.as_ref().map(|d| d.display().to_string()).into(),
                );
                out.insert(
                    "models".into(),
                    MODELS
                        .iter()
                        .map(|model| model_view(model, dir.as_deref()))
                        .collect(),
                );
                out.insert(
                    "speech_detector_installed".into(),
                    dir.as_deref()
                        .is_some_and(|dir| DETECTOR.installed(dir))
                        .into(),
                );
            }
            "install" => {
                let id = params
                    .model
                    .as_deref()
                    .unwrap_or(editor_media::speech::DEFAULT_MODEL);
                let model =
                    Model::by_id(id).ok_or_else(|| ToolError::invalid(format!("No model {id}")))?;
                let dir = dir.ok_or_else(|| {
                    ToolError::unavailable("There is no folder to keep models in")
                })?;
                let job =
                    self.shared
                        .jobs
                        .start("model_install", "Downloading", move |progress| {
                            std::fs::create_dir_all(&dir).map_err(|error| {
                                ToolError::new(codes::STORAGE, error.to_string())
                            })?;
                            match speech(
                                SpeechJob::Download {
                                    model,
                                    dir: dir.clone(),
                                },
                                progress,
                            )? {
                                SpeechEvent::Downloaded { .. } => Ok(model_view(model, Some(&dir))),
                                SpeechEvent::Failed { message, .. } => {
                                    Err(ToolError::new(codes::JOB_FAILED, message))
                                }
                                _ => Err(ToolError::new("cancelled", "Cancelled")),
                            }
                        })?;
                out.insert("job_id".into(), serde_json::json!(job.job_id));
                out.insert("job".into(), serde_json::to_value(job).unwrap_or_default());
            }
            other => {
                return Err(ToolError::invalid(format!(
                    "op is list or install, not {other}"
                )));
            }
        }
        Ok(Json(out))
    }

    #[tool(
        description = "Hear the timeline's speech with Whisper (a job): every word with its \
                          timeline time, and the language. Make captions of it with captions \
                          {op: from_transcript, job_id}."
    )]
    async fn transcribe_start(
        &self,
        Parameters(params): Parameters<TranscribeParams>,
    ) -> Result<Json<JobInfo>, ToolError> {
        let dir = self
            .shared
            .models
            .clone()
            .ok_or_else(|| ToolError::unavailable("There is no folder of models"))?;
        let id = params
            .model
            .as_deref()
            .unwrap_or(editor_media::speech::DEFAULT_MODEL);
        let model = Model::by_id(id).ok_or_else(|| ToolError::invalid(format!("No model {id}")))?;
        if !model.installed(&dir) {
            return Err(ToolError::new(
                codes::UNAVAILABLE,
                format!(
                    "The {id} model is not installed; transcription_models {{op: install, model: {id}}}"
                ),
            ));
        }
        let language = match params.language.as_deref() {
            None => None,
            Some(tag) => Some(
                editor_media::speech::whisper_language(tag)
                    .ok_or_else(|| ToolError::invalid(format!("Whisper does not know {tag}")))?
                    .to_owned(),
            ),
        };
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        check_readable(&snapshot.project, self.roots())?;
        let rate = snapshot.project.settings.frame_rate;
        let end = snapshot.project.duration();
        let from = params
            .from
            .as_ref()
            .map(|t| t.at(rate))
            .transpose()?
            .unwrap_or(Time::ZERO);
        let to = params
            .to
            .as_ref()
            .map(|t| t.at(rate))
            .transpose()?
            .unwrap_or(end)
            .min(end);
        if from >= to {
            return Err(ToolError::invalid("There is nothing to hear there"));
        }
        let project = snapshot.project.clone();
        let revision = snapshot.revision;
        let (project_id, context_id) = (project.id, snapshot.context_id);
        let job = Transcription {
            project: project.clone(),
            start: from,
            end: to,
            model: model.path(&dir),
            detector: Some(DETECTOR.path(&dir)),
            language,
            translate: params.translate,
        };
        self.shared
            .jobs
            .start("transcribe", "Listening", move |progress| {
                match speech(SpeechJob::Transcribe(job), progress)? {
                    SpeechEvent::Transcribed {
                        words, language, ..
                    } => {
                        let text = words
                            .iter()
                            .map(|word| word.text.trim())
                            .collect::<Vec<_>>()
                            .join(" ");
                        Ok(serde_json::json!({
                            "project_id": project_id,
                            "context_id": context_id,
                            "revision": revision,
                            "language": language,
                            "from": crate::mcp::time::Stamp::new(from, rate),
                            "to": crate::mcp::time::Stamp::new(to, rate),
                            "text": text,
                            "words": words.iter().map(|word| serde_json::json!({
                                "text": word.text,
                                "at": crate::mcp::time::Stamp::new(word.at, rate),
                                "end": crate::mcp::time::Stamp::new(word.end, rate),
                            })).collect::<Vec<_>>(),
                        }))
                    }
                    SpeechEvent::Failed { message, .. } => {
                        Err(ToolError::new(codes::JOB_FAILED, message))
                    }
                    _ => Err(ToolError::new("cancelled", "Cancelled")),
                }
            })
            .map(Json)
    }
}
