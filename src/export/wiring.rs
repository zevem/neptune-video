//! Where the application meets the export: what the export sheet opens
//! with, what a queued answer becomes, what quitting asks while jobs run,
//! how agent render operations are answered from the queue and how a
//! finished file is shown in the file manager.
//!
//! Everything here decides without the window, so it is tested without
//! one; the application calls it and does what it says.
use super::{
    engine::sidecar_for,
    presets::ExportPresets,
    queue::{EnqueueError, NewJob, RenderEvent, RenderQueue},
};
use editor_agent::{
    AgentOperation, AgentRequest, AgentResult, check_queue_export, codes, export_settings_schema,
};
use editor_core::{
    ClipId, Editor, EffectKind, Project, Time,
    export::{ExportPreset, ExportRange, ExportSettings},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

/// The name an export of an unnamed project is given.
pub const UNTITLED: &str = "Untitled";
/// The longest file stem suggested, in characters.
const LONGEST_STEM: usize = 120;

/// The presets offered: the built-in ones, then the person's saved ones.
pub fn offered_presets(saved: &ExportPresets) -> Vec<ExportPreset> {
    let mut presets = editor_core::export::builtin_presets();
    presets.extend(saved.presets().cloned());
    presets
}

/// The settings the sheet opens with: the last used, read leniently, or
/// the defaults. A field this build cannot read takes its default and the
/// others are kept; a container that cannot carry the picture or sound
/// read with it starts over from the defaults. The range is always the
/// whole timeline and the caption track the first (rule 18).
pub fn last_settings(value: Option<&serde_json::Value>) -> ExportSettings {
    let defaults = ExportSettings::default();
    let Some(value) = value else {
        return defaults;
    };
    let mut settings = match serde_json::from_value::<ExportSettings>(value.clone()) {
        Ok(settings) => settings,
        Err(_) => {
            let field = |name: &str| value.get(name).cloned();
            let mut settings = defaults.clone();
            if let Some(Ok(container)) = field("container").map(serde_json::from_value) {
                settings.container = container;
            }
            if let Some(Ok(video)) = field("video").map(serde_json::from_value) {
                settings.video = video;
            }
            if let Some(Ok(audio)) = field("audio").map(serde_json::from_value) {
                settings.audio = audio;
            }
            if let Some(Ok(subtitles)) = field("subtitles").map(serde_json::from_value) {
                settings.subtitles = subtitles;
            }
            settings
        }
    };
    let carried = settings.container.has_video() == settings.video.is_some()
        && (settings.video.is_some() || settings.audio.is_some());
    if !carried {
        settings = ExportSettings {
            subtitles: settings.subtitles,
            ..defaults
        };
    }
    forget_range(settings)
}

/// Settings without the range and caption track, as they are remembered.
pub fn forget_range(mut settings: ExportSettings) -> ExportSettings {
    settings.range = ExportRange::Timeline;
    settings.subtitles.track = None;
    settings
}

/// What `settings.json` keeps of the settings last queued.
pub fn remembered(settings: &ExportSettings) -> Option<serde_json::Value> {
    serde_json::to_value(forget_range(settings.clone())).ok()
}

/// A file stem from the project's name: characters no system takes are
/// left out, as are leading dots that would hide the file; an empty result
/// is [`UNTITLED`].
pub fn file_stem(project_name: &str) -> String {
    let kept: String = (project_name.chars())
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .take(LONGEST_STEM)
        .collect();
    let kept = kept.trim().trim_start_matches('.').trim();
    // Windows refuses names that end in a dot or a space.
    let kept = kept.trim_end_matches(['.', ' ']);
    if kept.is_empty() {
        UNTITLED.to_owned()
    } else {
        kept.to_owned()
    }
}

/// The folder the sheet suggests, first that is a folder: the one the
/// last export went to, the project file's, the person's videos folder,
/// then home.
pub fn initial_folder(
    last: Option<&Path>,
    project: Option<&Path>,
    videos: Option<PathBuf>,
    home: Option<PathBuf>,
    is_folder: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let project_folder = project.and_then(Path::parent).map(Path::to_path_buf);
    [last.map(Path::to_path_buf), project_folder, videos, home]
        .into_iter()
        .flatten()
        .find(|folder| folder.is_absolute() && is_folder(folder))
}

/// The span of the selected clips: the earliest start to the latest end.
pub fn selection_span(project: &Project, clips: &[ClipId]) -> Option<(Time, Time)> {
    let mut span: Option<(Time, Time)> = None;
    for (_, clip) in clips.iter().filter_map(|id| project.clip(*id)) {
        let (start, end) = (clip.start, clip.end());
        span = Some(match span {
            Some((from, to)) => (from.min(start), to.max(end)),
            None => (start, end),
        });
    }
    span.filter(|(start, end)| end > start)
}

/// What the export will draw differently from the preview, as far as is
/// known without reading anything: stabilized clips in the range whose
/// camera path `unsteady` says is missing.
pub fn warnings(
    project: &Project,
    settings: &ExportSettings,
    unsteady: impl Fn(&editor_core::Asset) -> bool,
) -> Vec<String> {
    let (start, end) = settings
        .span(project)
        .unwrap_or((Time::ZERO, project.duration()));
    let mut named = Vec::new();
    for track in project.tracks.iter().filter(|track| !track.hidden) {
        for clip in &track.clips {
            if clip.end() <= start || clip.start >= end {
                continue;
            }
            let steadied = (clip.effects.iter())
                .any(|effect| effect.enabled && effect.kind == EffectKind::Stabilize);
            let asset = clip.asset_id().and_then(|id| project.assets.get(&id));
            if let Some(asset) = asset
                && steadied
                && unsteady(asset)
                && !named.contains(&asset.name)
            {
                named.push(asset.name.clone());
            }
        }
    }
    (named.into_iter())
        .map(|name| {
            format!("{name} is stabilized but not analysed; it exports as the camera took it")
        })
        .collect()
}

/// The job a Queue answer of the sheet asks for. `start_now` is Export;
/// Add to queue queues it paused.
pub fn new_job(
    project: Arc<Project>,
    context: Uuid,
    revision: u64,
    settings: ExportSettings,
    destination: PathBuf,
    start_now: bool,
    project_path: Option<PathBuf>,
) -> NewJob {
    NewJob {
        project,
        context,
        revision,
        settings,
        destination,
        start_paused: !start_now,
        project_path,
    }
}

/// The files an export would replace: the destination and the caption file
/// beside it, those of them `exists` finds. The person is asked about every
/// one before the export is queued.
pub fn replaced_files(
    project: &Project,
    settings: &ExportSettings,
    destination: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    std::iter::once(destination.to_path_buf())
        .chain(sidecar_for(project, settings, destination))
        .filter(|path| exists(path))
        .collect()
}

/// Whether a file, not a folder, is at `path`: what replacing asks about.
pub fn file_there(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_dir())
}

/// What closing the window asks first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseQuestion {
    /// Nothing: it closes.
    None,
    /// Jobs are rendering or waiting: "Rendering will stop".
    Rendering,
    /// The project has unsaved changes.
    Unsaved,
}

/// The next question before the window closes. Rendering is asked first,
/// because keeping it ends the quit; unsaved work is asked once stopping
/// was agreed to, or when nothing renders.
pub fn close_question(busy: bool, stop_agreed: bool, dirty: bool) -> CloseQuestion {
    if busy && !stop_agreed {
        CloseQuestion::Rendering
    } else if dirty {
        CloseQuestion::Unsaved
    } else {
        CloseQuestion::None
    }
}

/// Cancels every unfinished job at once, as Stop and quit asks: the
/// waiting ones first, so that none starts as the one rendering stops, then
/// the one under way, whose processes are killed and whose unfinished files
/// are removed. The jobs stay listed, cancelled, to be retried should the
/// quit not go ahead. Returns how many were cancelled.
pub fn stop_all(queue: &RenderQueue) -> usize {
    let mut stopped = 0;
    for id in queue.waiting() {
        stopped += usize::from(queue.cancel(id));
    }
    // What is left: the job under way, and any that started meanwhile.
    for job in queue.jobs() {
        if !job.state.is_finished() {
            stopped += usize::from(queue.cancel(job.id));
        }
    }
    stopped
}

/// The notice a finished job is told with, and whether it tells of a
/// problem. None while the render queue is open (`queue_open`): its row
/// says it, Done or Failed with the reason.
pub fn finished_notice(event: &RenderEvent, queue_open: bool) -> Option<(String, bool)> {
    if queue_open {
        return None;
    }
    let RenderEvent::Finished {
        name, result, note, ..
    } = event;
    Some(match (result, note) {
        (Ok(_), None) => (format!("Exported {name}"), false),
        (Ok(_), Some(note)) => (format!("Exported {name}: {note}"), false),
        (Err(why), _) => (format!("{name} was not exported: {why}"), true),
    })
}

/// An enqueue refusal as the agent sees it.
pub fn enqueue_error(error: &EnqueueError) -> AgentResult {
    match error {
        EnqueueError::Invalid(error) => AgentResult::export_error(error),
        other => AgentResult::error(other.code(), other.to_string()),
    }
}

/// The answer to a render operation, from the queue and never blocking on
/// a job: capabilities as probed, `queue_export` after the identity guards
/// and the destination check, the job operations by id. `presets` lists the
/// built-in and saved presets; it is asked for only by
/// `export_capabilities`. `queue_export` snapshots the editor as it is: the
/// agent's revision does not include the person's uncommitted drafts.
pub fn render_result(
    queue: &RenderQueue,
    editor: &Editor,
    request: &AgentRequest,
    presets: impl FnOnce() -> Vec<ExportPreset>,
    project_path: Option<PathBuf>,
) -> AgentResult {
    use super::queue::CapabilityState;
    if let Some(result) = render_job_result(queue, &request.operation) {
        return result;
    }
    let job = |id: Uuid| match queue.job(id) {
        Some(job) => AgentResult::RenderJob { job: Box::new(job) },
        None => unknown(id, None),
    };
    match &request.operation {
        AgentOperation::ExportCapabilities => match queue.capabilities() {
            CapabilityState::Probing => pending(),
            CapabilityState::Failed(why) => AgentResult::error(codes::UNSUPPORTED_EXPORT, why),
            CapabilityState::Ready(caps) => AgentResult::ExportCapabilities {
                capabilities: Box::new((*caps).clone()),
                presets: presets(),
                settings_schema: export_settings_schema(),
            },
        },
        AgentOperation::QueueExport {
            settings,
            destination,
            overwrite,
        } => {
            if let Err(refused) = check_queue_export(editor, request) {
                return refused;
            }
            // The caption file beside it is replaced only with the same
            // consent as the file itself.
            if !overwrite
                && let Some(sidecar) = sidecar_for(editor.project(), settings, destination)
                && file_there(&sidecar)
            {
                return AgentResult::error(
                    codes::DESTINATION_EXISTS,
                    format!(
                        "{} exists; send overwrite: true to replace it",
                        sidecar.display()
                    ),
                );
            }
            let new = NewJob {
                project: editor.snapshot(),
                context: editor.instance_id(),
                revision: editor.revision(),
                settings: settings.clone(),
                destination: destination.clone(),
                start_paused: false,
                project_path,
            };
            match queue.enqueue(new) {
                Ok(id) => job(id),
                Err(error) => enqueue_error(&error),
            }
        }
        AgentOperation::RenderJobs
        | AgentOperation::PauseRender { .. }
        | AgentOperation::ResumeRender { .. }
        | AgentOperation::CancelRender { .. }
        | AgentOperation::RetryRender { .. }
        | AgentOperation::RemoveRender { .. }
        | AgentOperation::Inspect
        | AgentOperation::Apply { .. }
        | AgentOperation::Undo
        | AgentOperation::Redo
        | AgentOperation::Capabilities
        | AgentOperation::History
        | AgentOperation::Document
        | AgentOperation::Save { .. } => AgentResult::error(
            codes::UNSUPPORTED,
            "Only render operations are answered from the render queue",
        ),
    }
}

/// The operations on jobs already in `queue`, which need no project:
/// listing them, and pausing, resuming, cancelling, retrying or removing one.
/// `None` for every other operation.
pub fn render_job_result(queue: &RenderQueue, operation: &AgentOperation) -> Option<AgentResult> {
    let job = |id: Uuid| match queue.job(id) {
        Some(job) => AgentResult::RenderJob { job: Box::new(job) },
        None => unknown(id, None),
    };
    // A job that is there but whose state refuses the operation.
    let refused = |id: Uuid, what: &str| match queue.job(id) {
        Some(found) => unknown(id, Some((what, found.state))),
        None => unknown(id, None),
    };
    Some(match operation {
        AgentOperation::RenderJobs => AgentResult::RenderJobs { jobs: queue.jobs() },
        AgentOperation::PauseRender { job: id } => match queue.pause(*id) {
            true => job(*id),
            false => refused(*id, "paused"),
        },
        AgentOperation::ResumeRender { job: id } => match queue.resume(*id) {
            true => job(*id),
            false => refused(*id, "resumed"),
        },
        AgentOperation::CancelRender { job: id } => match queue.cancel(*id) {
            true => job(*id),
            false => refused(*id, "cancelled"),
        },
        AgentOperation::RetryRender { job: id } => match queue.retry(*id) {
            true => job(*id),
            false => refused(*id, "retried"),
        },
        AgentOperation::RemoveRender { job: id } => {
            // Answered with the job as it was before it went.
            let before = queue.job(*id);
            match (before, queue.remove(*id)) {
                (Some(before), true) => AgentResult::RenderJob {
                    job: Box::new(before),
                },
                _ => refused(*id, "removed while it renders"),
            }
        }
        _ => return None,
    })
}

fn pending() -> AgentResult {
    AgentResult::error(
        codes::CAPABILITIES_PENDING,
        "FFmpeg's encoders are still being checked; ask again in a moment",
    )
}

fn unknown(id: Uuid, refused: Option<(&str, editor_core::export::RenderState)>) -> AgentResult {
    let message = match refused {
        Some((what, state)) => {
            let state = serde_json::to_value(state)
                .ok()
                .and_then(|state| state.as_str().map(str::to_owned))
                .unwrap_or_default();
            format!("Render job {id} is {state} and cannot be {what}")
        }
        None => format!("There is no render job {id}; list render_jobs for the current ones"),
    };
    AgentResult::error(codes::UNKNOWN_JOB, message)
}

/// The program and arguments that show `file` in the system's file
/// manager: the file selected where that is possible, otherwise its folder.
/// On Windows the argument is given as it is, unquoted (see
/// [`explorer_select`]).
pub fn reveal_command(file: &Path) -> (String, Vec<std::ffi::OsString>) {
    if cfg!(target_os = "macos") {
        (
            "open".into(),
            vec!["-R".into(), file.as_os_str().to_owned()],
        )
    } else if cfg!(target_os = "windows") {
        ("explorer".into(), vec![explorer_select(file)])
    } else {
        let folder = file.parent().unwrap_or(file);
        ("xdg-open".into(), vec![folder.as_os_str().to_owned()])
    }
}

/// Explorer's "select this file": the switch and the path, quoted, in one
/// argument. Explorer reads its command line itself and ignores a switch
/// quoted whole, as the usual quoting of an argument with a space would
/// have it, so this is passed as it is (`raw_arg`). Windows paths hold no
/// quotes.
pub fn explorer_select(file: &Path) -> std::ffi::OsString {
    let mut select = std::ffi::OsString::from("/select,\"");
    select.push(file.as_os_str());
    select.push("\"");
    select
}

/// Shows `file` in the file manager without waiting: the program is
/// started and reaped on a thread of its own. Says why it could not start.
pub fn reveal(file: &Path) -> Result<(), String> {
    let (program, args) = reveal_command(file);
    let mut command = std::process::Command::new(&program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        for arg in &args {
            command.raw_arg(arg);
        }
    }
    #[cfg(not(windows))]
    command.args(&args);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("The file manager could not be opened ({program}: {error})"))?;
    std::thread::Builder::new()
        .name("aster-reveal".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map(|_| ())
        .map_err(|error| format!("The file manager could not be opened ({error})"))
}

#[cfg(test)]
#[path = "wiring_tests.rs"]
mod tests;
