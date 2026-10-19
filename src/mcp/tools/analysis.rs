//! Looking and listening: frames and contact sheets of the timeline or of
//! one media file, the cuts between shots, loudness, and speech heard with
//! word times. Pictures come back inline; long work runs as jobs whose
//! results the agent applies with ordinary tools.
use super::{check_readable, clip, resolve_asset_id};
use crate::mcp::{
    errors::{ToolError, codes},
    frames::{self, Drawn, MOST_FRAMES},
    jobs::{JobInfo, Progress},
    server::NeptuneVideoServer,
    time::{Stamp, TimeSpec},
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
    /// Exact timeline times (source times with asset_id); seconds/us do
    /// not snap to the project frame grid.
    pub times: Vec<TimeSpec>,
    /// Draw a media file's own frames instead of the timeline.
    #[serde(default)]
    pub asset_id: Option<String>,
    #[serde(flatten)]
    pub selection: RenderSelection,
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
    /// Exact times in order; replaces evenly spaced from/to/count.
    #[serde(default)]
    pub times: Option<Vec<TimeSpec>>,
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
    pub asset_id: Option<String>,
    #[serde(flatten)]
    pub selection: RenderSelection,
    #[serde(default)]
    pub save_to: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

/// A temporary picture selection, never an edit to track visibility.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RenderSelection {
    /// Draw just these tracks, including hidden tracks.
    #[serde(default)]
    pub only_tracks: Vec<Uuid>,
    /// Leave these tracks out of the picture.
    #[serde(default)]
    pub exclude_tracks: Vec<Uuid>,
    /// Draw this clip alone, including its transform and effects.
    #[serde(default)]
    pub solo_clip: Option<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FramesExportParams {
    /// Folder for frame-000001.png (or .jpg) and subsequent frames.
    pub destination: String,
    #[serde(default)]
    pub from: Option<TimeSpec>,
    /// Exclusive end; defaults to the end of the timeline or source.
    #[serde(default)]
    pub to: Option<TimeSpec>,
    /// Sampling interval; defaults to one project/source frame.
    #[serde(default)]
    pub step: Option<TimeSpec>,
    #[serde(default)]
    pub asset_id: Option<String>,
    #[serde(flatten)]
    pub selection: RenderSelection,
    /// Longest side, 1280 by default, at most 2048.
    #[serde(default)]
    pub size: Option<u32>,
    /// png (default) or jpeg.
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScenesParams {
    /// A video in the project.
    #[serde(default)]
    pub asset_id: Option<String>,
    /// Or a clip: its media between its in and out, with timeline times.
    #[serde(default)]
    pub clip_id: Option<Uuid>,
    /// 0 to 1; 0.35 by default. Lower finds softer cuts.
    #[serde(default)]
    pub threshold: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoudnessParams {
    /// `clips` (default), or `timeline`/`mix`: the export's post-limiter mix.
    #[serde(default)]
    pub scope: Option<String>,
    /// Clips whose sound to measure, with their audio effects.
    #[serde(default)]
    pub clip_ids: Vec<Uuid>,
    /// Mix: include only these tracks, retaining existing mute/solo rules.
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
    #[serde(default)]
    pub from: Option<TimeSpec>,
    #[serde(default)]
    pub to: Option<TimeSpec>,
    /// Also measure each selected track alone through the master stage.
    #[serde(default)]
    pub per_track: bool,
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
fn subject(
    project: &Arc<Project>,
    asset: Option<&str>,
    selection: &RenderSelection,
) -> Result<Arc<Project>, ToolError> {
    let selecting = !selection.only_tracks.is_empty()
        || !selection.exclude_tracks.is_empty()
        || selection.solo_clip.is_some();
    if asset.is_some() && selecting {
        return Err(ToolError::invalid(
            "asset_id cannot be combined with track or clip selection",
        ));
    }
    match asset {
        None if !selecting => Ok(project.clone()),
        None => {
            for id in selection
                .only_tracks
                .iter()
                .chain(&selection.exclude_tracks)
            {
                super::track(project, *id)?;
            }
            let solo_track = selection
                .solo_clip
                .map(|id| clip(project, id).map(|(track, _)| track.id))
                .transpose()?;
            if let Some(id) = solo_track
                && (selection.exclude_tracks.contains(&id)
                    || (!selection.only_tracks.is_empty() && !selection.only_tracks.contains(&id)))
            {
                return Err(ToolError::invalid(
                    "solo_clip is outside the selected tracks",
                ));
            }
            let mut preview = (**project).clone();
            preview.tracks.retain(|track| {
                (selection.only_tracks.is_empty() || selection.only_tracks.contains(&track.id))
                    && !selection.exclude_tracks.contains(&track.id)
                    && solo_track.is_none_or(|id| track.id == id)
            });
            for track in &mut preview.tracks {
                if solo_track.is_some() || selection.only_tracks.contains(&track.id) {
                    let track = Arc::make_mut(track);
                    track.hidden = false;
                    if let Some(id) = selection.solo_clip {
                        track.clips.retain(|clip| clip.id == id);
                    }
                }
            }
            // Keep fonts and LUTs, but unrelated offline media must not
            // prevent looking at the selected layers.
            let used: std::collections::HashSet<_> = preview
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .flat_map(|clip| {
                    clip.asset_id()
                        .into_iter()
                        .chain(clip.effects.iter().filter_map(|effect| effect.asset_id))
                })
                .collect();
            Arc::make_mut(&mut preview.assets)
                .retain(|id, asset| used.contains(id) || asset.kind.is_resource());
            Ok(Arc::new(preview))
        }
        Some(given) => {
            let id = resolve_asset_id(project, given)?;
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

fn jpeg_format(format: Option<&str>) -> Result<bool, ToolError> {
    match format {
        None | Some("png") => Ok(false),
        Some("jpeg" | "jpg") => Ok(true),
        Some(other) => Err(ToolError::invalid(format!(
            "format is png or jpeg, not {other}"
        ))),
    }
}

fn sequence_path(folder: &std::path::Path, index: usize, jpeg: bool) -> PathBuf {
    folder.join(format!(
        "frame-{:06}.{}",
        index + 1,
        if jpeg { "jpg" } else { "png" }
    ))
}

#[derive(Clone, Copy)]
enum SequenceStep {
    Frames(i64),
    Time(Time),
}

fn sequence_times(
    from: Time,
    to: Time,
    step: SequenceStep,
    rate: editor_core::FrameRate,
) -> Result<Vec<Time>, ToolError> {
    const MOST_DUMP_FRAMES: usize = 100_000;
    if from >= to
        || match step {
            SequenceStep::Frames(n) => n <= 0,
            SequenceStep::Time(t) => t <= Time::ZERO,
        }
    {
        return Err(ToolError::invalid(
            "from must precede to and step must be positive",
        ));
    }
    let mut times = Vec::new();
    for index in 0..=MOST_DUMP_FRAMES {
        let offset = match step {
            SequenceStep::Frames(frames) => rate.frame_time(frames.saturating_mul(index as i64)),
            SequenceStep::Time(time) => Time(time.0.saturating_mul(index as i64)),
        };
        let at = Time(from.0.saturating_add(offset.0));
        if at >= to {
            return Ok(times);
        }
        if index == MOST_DUMP_FRAMES {
            return Err(ToolError::new(
                codes::TOO_LARGE,
                "A frame dump supports at most 100000 frames",
            ));
        }
        times.push(at);
    }
    Err(ToolError::new(
        codes::TOO_LARGE,
        "A frame dump supports at most 100000 frames",
    ))
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
impl NeptuneVideoServer {
    #[tool(
        description = "Draw exact timeline/source times with export effects and titles. \
                       only_tracks, exclude_tracks or solo_clip isolate layers without editing \
                       visibility. Use frames_export for long image sequences.",
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
        if params.no_inline && params.save_to.is_none() {
            return Err(ToolError::invalid("no_inline requires save_to"));
        }
        let jpeg = jpeg_format(params.format.as_deref())?;
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let drawn_of = subject(
            &snapshot.project,
            params.asset_id.as_deref(),
            &params.selection,
        )?;
        check_readable(&drawn_of, self.roots())?;
        let rate = drawn_of.settings.frame_rate;
        let times = params
            .times
            .iter()
            .map(|time| time.exact(rate))
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
        for path in &save {
            self.roots().create_parent(path)?;
        }
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
            "time_basis": if params.asset_id.is_some() { "source" } else { "timeline" },
            "frames": drawn.iter().map(|picture| serde_json::json!({
                "at": Stamp::new(picture.at, rate),
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
        description = "One picture of frames at explicit times or evenly spaced over the \
                       timeline or a media file: an overview of the edit.",
        annotations(read_only_hint = true)
    )]
    async fn contact_sheet(
        &self,
        Parameters(params): Parameters<SheetParams>,
    ) -> Result<CallToolResult, ToolError> {
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let drawn_of = subject(
            &snapshot.project,
            params.asset_id.as_deref(),
            &params.selection,
        )?;
        check_readable(&drawn_of, self.roots())?;
        let rate = drawn_of.settings.frame_rate;
        let end = drawn_of.duration();
        if end <= Time::ZERO {
            return Err(ToolError::invalid(
                "There is nothing on the timeline to draw",
            ));
        }
        let times = if let Some(times) = &params.times {
            if params.from.is_some() || params.to.is_some() || params.count.is_some() {
                return Err(ToolError::invalid("Give times or from/to/count, not both"));
            }
            if times.is_empty() || times.len() > MOST_FRAMES {
                return Err(ToolError::invalid(format!(
                    "Ask for 1 to {MOST_FRAMES} times"
                )));
            }
            times
                .iter()
                .map(|time| time.exact(rate))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let first = params
                .selection
                .solo_clip
                .map(|id| clip(&drawn_of, id).map(|(_, clip)| clip.start))
                .transpose()?
                .unwrap_or_default();
            let from = params
                .from
                .as_ref()
                .map(|t| t.exact(rate))
                .transpose()?
                .unwrap_or(first);
            let to = params
                .to
                .as_ref()
                .map(|t| t.exact(rate))
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
            times
        };
        let count = times.len();
        let save = match &params.save_to {
            Some(given) => Some(
                self.roots()
                    .destination(std::path::Path::new(given), params.overwrite)?,
            ),
            None => None,
        };
        if let Some(path) = &save {
            self.roots().create_parent(path)?;
        }
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
            "time_basis": if params.asset_id.is_some() { "source" } else { "timeline" },
            "width": width,
            "height": height,
            "columns": columns,
            "cells": drawn.iter().enumerate().map(|(index, picture)| serde_json::json!({
                "index": index,
                "at": Stamp::new(picture.at, rate),
            })).collect::<Vec<_>>(),
            "saved": save.map(|path| path.display().to_string()),
        });
        Ok(with_pictures(value, vec![(bytes, "image/jpeg")]))
    }

    #[tool(
        description = "Write a range as numbered PNG/JPEG frames (a job), one frame at a time \
                       with bounded memory. Supports source media and isolated layers; creates \
                       the output folder. Completed files remain on cancellation. Poll with job."
    )]
    async fn frames_export(
        &self,
        Parameters(params): Parameters<FramesExportParams>,
    ) -> Result<Json<JobInfo>, ToolError> {
        let jpeg = jpeg_format(params.format.as_deref())?;
        let snapshot = self.on(|workspace| workspace.snapshot()).await?;
        let drawn_of = subject(
            &snapshot.project,
            params.asset_id.as_deref(),
            &params.selection,
        )?;
        check_readable(&drawn_of, self.roots())?;
        let rate = drawn_of.settings.frame_rate;
        let first = params
            .selection
            .solo_clip
            .map(|id| clip(&drawn_of, id).map(|(_, clip)| clip.start))
            .transpose()?
            .unwrap_or_default();
        let from = params
            .from
            .as_ref()
            .map(|time| time.exact(rate))
            .transpose()?
            .unwrap_or(first);
        let to = params
            .to
            .as_ref()
            .map(|time| time.exact(rate))
            .transpose()?
            .unwrap_or_else(|| drawn_of.duration())
            .min(drawn_of.duration());
        let step = match params.step.as_ref() {
            None => SequenceStep::Frames(1),
            Some(given) => {
                let exact = given.exact(rate)?;
                if let Some(frames) = given.frame {
                    SequenceStep::Frames(frames)
                } else if given.timecode.is_some() {
                    SequenceStep::Frames(rate.frame_at(exact + Time(1)))
                } else {
                    SequenceStep::Time(exact)
                }
            }
        };
        let times = sequence_times(from, to, step, rate)?;
        let first = self.roots().destination(
            &sequence_path(std::path::Path::new(&params.destination), 0, jpeg),
            params.overwrite,
        )?;
        let folder = first
            .parent()
            .ok_or_else(|| ToolError::invalid("The output folder is invalid"))?
            .to_path_buf();
        let roots = self.shared.roots.clone();
        let frames = self.shared.frames.clone();
        let revision = snapshot.revision;
        let context_id = snapshot.context_id;
        let project_id = snapshot.project.id;
        self.shared.jobs.start("frames_export", "Preparing frame dump", move |progress| {
            // Check the entire sequence before producing a picture, so an
            // existing later frame cannot cause a partial overwrite.
            for index in 0..times.len() {
                if progress.cancelled() {
                    return Err(ToolError::new("cancelled", "Cancelled"));
                }
                roots.destination(&sequence_path(&folder, index, jpeg), params.overwrite)?;
            }
            roots.create_parent(&first)?;
            let stream = loop {
                if progress.cancelled() {
                    return Err(ToolError::new("cancelled", "Cancelled"));
                }
                match frames.stream(drawn_of.clone(), times.clone(), params.size.unwrap_or(1280)) {
                    Ok(stream) => break stream,
                    Err(error) if error.code == codes::BUSY => {
                        progress.set(0.0, "Waiting for the frame renderer");
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) => return Err(error),
                }
            };
            let mut size = [0, 0];
            for index in 0..times.len() {
                let picture = loop {
                    if progress.cancelled() {
                        return Err(ToolError::new("cancelled", "Cancelled"));
                    }
                    match stream.recv_timeout(Duration::from_millis(100)) {
                        Ok(picture) => break picture?,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(ToolError::unavailable("The frame renderer stopped before the sequence finished"));
                        }
                    }
                };
                size = [picture.width, picture.height];
                let encoded = if jpeg {
                    frames::jpeg(picture.width, picture.height, &picture.rgba)?
                } else {
                    frames::png(&picture)?
                };
                let path = roots.destination(&sequence_path(&folder, index, jpeg), params.overwrite)?;
                write_picture(&path, &encoded)?;
                progress.set((index + 1) as f32 / times.len() as f32,
                    format!("Wrote {} of {} frames", index + 1, times.len()));
            }
            Ok(serde_json::json!({
                "project_id": project_id, "context_id": context_id, "revision": revision,
                "destination": folder, "count": times.len(), "width": size[0], "height": size[1],
                "format": if jpeg { "jpeg" } else { "png" },
                "pattern": if jpeg { "frame-%06d.jpg" } else { "frame-%06d.png" },
                "time_basis": if params.asset_id.is_some() { "source" } else { "timeline" },
                "from": Stamp::new(from, rate), "to": Stamp::new(to, rate),
                "step_us": match step { SequenceStep::Time(time) => Some(time.0), _ => None },
                "step_frames": match step { SequenceStep::Frames(frames) => Some(frames), _ => None },
                "frame_rate": rate,
            }))
        }).map(Json)
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
            (Some(given), None) => {
                let id = super::resolve_asset_id(&project, &given)?;
                (
                    project
                        .assets
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| ToolError::not_found(format!("No media {id}")))?,
                    None,
                )
            }
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
        description = "Measure LUFS and true peak as a job: clips (for audio_edit normalize), \
                          or scope: timeline for the export mix, with optional per_track stats."
    )]
    async fn loudness_start(
        &self,
        Parameters(params): Parameters<LoudnessParams>,
    ) -> Result<Json<JobInfo>, ToolError> {
        match params.scope.as_deref().unwrap_or("clips") {
            "timeline" | "mix" => return super::mix::start(self, params).await.map(Json),
            "clips" => {}
            _ => return Err(ToolError::invalid("scope is clips, timeline or mix")),
        }
        if !params.track_ids.is_empty()
            || params.from.is_some()
            || params.to.is_some()
            || params.per_track
        {
            return Err(ToolError::invalid(
                "track_ids, from, to and per_track require scope: timeline",
            ));
        }
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

#[cfg(test)]
mod picture_tests {
    use super::*;
    use editor_core::{Asset, Clip, Effect, EffectKind, FrameRate, MediaKind, Track, TrackKind};

    #[test]
    fn isolation_reveals_the_requested_clip_without_changing_the_project() {
        let mut project = Project::new("Layers");
        let mut asset = Asset::new(
            "picture.png".into(),
            MediaKind::Image,
            Time::from_seconds(2.0),
        );
        asset.width = 160;
        asset.height = 90;
        let mut placed = Clip::from_asset(&asset, Time::ZERO);
        placed.transform.position = [25.0, 10.0];
        placed.effects.push(Effect::new(EffectKind::Grade));
        let expected = placed.clone();
        let mut hidden = Track::new("Presenter", TrackKind::Video);
        hidden.hidden = true;
        hidden.clips.push(placed);
        hidden
            .clips
            .push(Clip::from_asset(&asset, Time::from_seconds(2.0)));
        project.tracks = vec![
            Arc::new(hidden),
            Arc::new(Track::new("Overlay", TrackKind::Video)),
        ];
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        let unused = Asset::new(
            "offline.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(1.0),
        );
        Arc::make_mut(&mut project.assets).insert(unused.id, unused);
        let original = Arc::new(project);
        let preview = subject(
            &original,
            None,
            &RenderSelection {
                solo_clip: Some(expected.id),
                ..RenderSelection::default()
            },
        )
        .unwrap();
        assert_eq!(preview.tracks.len(), 1);
        assert_eq!(preview.tracks[0].clips, vec![expected]);
        assert!(!preview.tracks[0].hidden);
        assert_eq!(preview.assets.len(), 1);
        assert!(original.tracks[0].hidden);
        assert_eq!(original.tracks[0].clips.len(), 2);
        assert_eq!(original.assets.len(), 2);
    }

    #[test]
    fn picture_subject_uses_source_rate_and_rejects_conflicting_filters() {
        let mut project = Project::new("Thirty");
        let mut asset = Asset::new(
            "source.mp4".into(),
            editor_core::MediaKind::Video,
            Time::from_seconds(2.0),
        );
        asset.width = 160;
        asset.height = 90;
        asset.frame_rate = Some(FrameRate {
            numerator: 60,
            denominator: 1,
        });
        let id = asset.id;
        Arc::make_mut(&mut project.assets).insert(id, asset);
        let project = Arc::new(project);
        let preview =
            subject(&project, Some(&id.to_string()), &RenderSelection::default()).unwrap();
        assert_eq!(preview.settings.frame_rate.fps(), 60.0);
        assert_eq!(
            TimeSpec::seconds(0.016667)
                .exact(preview.settings.frame_rate)
                .unwrap(),
            Time(16_667)
        );
        let error = subject(
            &project,
            Some(&id.to_string()),
            &RenderSelection {
                only_tracks: vec![project.tracks[0].id],
                ..RenderSelection::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.code, codes::INVALID);
    }

    #[test]
    fn frame_dump_keeps_rational_frame_positions_and_a_half_open_range() {
        let rate = editor_core::FrameRate {
            numerator: 60_000,
            denominator: 1001,
        };
        let from = Time(12_345);
        let times = sequence_times(
            from,
            from + rate.frame_time(60_000),
            SequenceStep::Frames(1),
            rate,
        )
        .unwrap();
        assert_eq!(times.len(), 60_000);
        assert_eq!(times[59_999], from + rate.frame_time(59_999));
        assert_eq!(
            sequence_times(Time::ZERO, Time(10), SequenceStep::Time(Time(3)), rate).unwrap(),
            vec![Time(0), Time(3), Time(6), Time(9)]
        );
        assert!(
            sequence_times(Time::ZERO, Time(10), SequenceStep::Time(Time::ZERO), rate).is_err()
        );
        assert_eq!(
            sequence_times(
                Time::ZERO,
                rate.frame_time(100_001),
                SequenceStep::Frames(1),
                rate
            )
            .unwrap_err()
            .code,
            codes::TOO_LARGE
        );
    }
}
