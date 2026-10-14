//! The decisions between the application and the export, without a window:
//! the sheet's starting point, the job a queued answer makes, the close
//! guard, and every agent render operation answered from a real queue whose
//! probe and job runner are fakes (no FFmpeg, no device).
use super::*;
use crate::export::{
    engine::Finish,
    queue::{CapabilityState, Probe, Runner},
};
use editor_core::{
    Asset, Clip, FrameRate, MediaKind, TrackId, TrackKind,
    export::{
        AudioEncoder, Capabilities, Container, RenderState, SidecarFormat, VideoCodec,
        VideoEncoder, known_video_encoder,
    },
};
use std::{
    fs,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const PATIENCE: Duration = Duration::from_secs(5);

/// A folder of the test's own, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("aster-wiring-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("FIXTURE: the scratch folder is made");
        Self(path.canonicalize().expect("FIXTURE: the folder resolves"))
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(
            Instant::now() < deadline,
            "FIXTURE: waited too long for {what}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn caps() -> Capabilities {
    let x264 = known_video_encoder("libx264").expect("libx264 is a known encoder");
    Capabilities {
        ffmpeg: "ffmpeg version test".into(),
        video: vec![VideoEncoder {
            name: x264.name.into(),
            codec: x264.codec,
            hardware: None,
            max_side: x264.max_side,
            min_size: x264.min_size,
            device: None,
        }],
        audio: vec![AudioEncoder {
            name: "aac".into(),
            codec: editor_core::export::AudioCodec::Aac,
        }],
        containers: Container::ALL.to_vec(),
        unavailable: Vec::new(),
    }
}

/// A job that renders until the test lets it finish, or until cancelled.
fn runner(finish: Arc<AtomicBool>) -> Runner {
    Box::new(move |ticket, handle| {
        handle.phase(RenderState::Rendering);
        loop {
            if handle.cancelled() || handle.hold().is_none() {
                return Finish::Cancelled;
            }
            if finish.load(Ordering::SeqCst) {
                return Finish::Done {
                    path: ticket.destination.clone(),
                    bytes: Some(1),
                    note: None,
                };
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })
}

struct Rig {
    queue: RenderQueue,
    editor: Editor,
    finish: Arc<AtomicBool>,
    dir: Scratch,
}

/// An editor with one video clip and a queue whose probe answers `probe`.
fn rig_with(probe: Probe) -> Rig {
    let dir = Scratch::new();
    let source = dir.join("source.mp4");
    fs::write(&source, b"not really a video").unwrap();
    let mut asset = Asset::new(source, MediaKind::Video, Time::from_seconds(2.0));
    asset.width = 320;
    asset.height = 180;
    asset.frame_rate = Some(FrameRate::default());
    let mut project = Project::new("Wiring test");
    project.settings.width = 320;
    project.settings.height = 180;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let track = (project.tracks.iter_mut())
        .find(|track| track.kind == TrackKind::Video)
        .expect("FIXTURE: a new project has a video track");
    Arc::make_mut(track)
        .clips
        .push(Clip::from_asset(&asset, Time::ZERO));
    let finish = Arc::new(AtomicBool::new(false));
    let queue = RenderQueue::with_parts(|| {}, probe, runner(Arc::clone(&finish)));
    Rig {
        queue,
        editor: Editor::new(project).expect("FIXTURE: the project is valid"),
        finish,
        dir,
    }
}

fn rig() -> Rig {
    let rig = rig_with(Arc::new(|| Ok(caps())));
    until("the capabilities", || {
        matches!(rig.queue.capabilities(), CapabilityState::Ready(_))
    });
    rig
}

impl Rig {
    fn request(&self, operation: AgentOperation) -> AgentRequest {
        AgentRequest {
            request_id: "r".into(),
            expected_revision: Some(self.editor.revision()),
            expected_project_id: Some(self.editor.project().id),
            expected_context_id: Some(self.editor.instance_id()),
            operation,
        }
    }

    fn answer(&self, operation: AgentOperation) -> AgentResult {
        let request = self.request(operation);
        render_result(&self.queue, &self.editor, &request, Vec::new, None)
    }

    fn queue_export(&self, name: &str, overwrite: bool) -> AgentResult {
        self.answer(AgentOperation::QueueExport {
            settings: ExportSettings::default(),
            destination: self.dir.join(name),
            overwrite,
        })
    }
}

#[track_caller]
fn code(result: &AgentResult) -> &str {
    match result {
        AgentResult::Error { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[track_caller]
fn job(result: AgentResult) -> editor_core::export::RenderJobInfo {
    match result {
        AgentResult::RenderJob { job } => *job,
        other => panic!("expected a job, got {other:?}"),
    }
}

#[test]
fn the_last_settings_are_read_leniently_without_range_or_track() {
    assert_eq!(last_settings(None), ExportSettings::default());
    assert_eq!(
        last_settings(Some(&serde_json::json!(12))),
        ExportSettings::default()
    );

    let mut kept = ExportSettings {
        container: Container::Mov,
        range: ExportRange::Span {
            start: Time::ZERO,
            end: Time::from_seconds(1.0),
        },
        ..ExportSettings::default()
    };
    kept.video.as_mut().unwrap().codec = VideoCodec::ProRes;
    kept.subtitles.sidecar = Some(SidecarFormat::Srt);
    kept.subtitles.track = Some(TrackId::new_v4());
    let read = last_settings(remembered(&kept).as_ref());
    assert_eq!(read.container, Container::Mov);
    assert_eq!(read.video.unwrap().codec, VideoCodec::ProRes);
    assert_eq!(read.subtitles.sidecar, Some(SidecarFormat::Srt));
    assert_eq!(read.range, ExportRange::Timeline, "the range is not kept");
    assert_eq!(read.subtitles.track, None, "nor is the caption track");
    // Even as written by hand with them.
    let written = serde_json::to_value(&kept).unwrap();
    let read = last_settings(Some(&written));
    assert_eq!(read.range, ExportRange::Timeline);
    assert_eq!(read.subtitles.track, None);

    // A picture this build cannot read takes the default; the rest stays.
    let mut later = serde_json::to_value(&kept).unwrap();
    later["video"]["codec"] = serde_json::json!("av2");
    let read = last_settings(Some(&later));
    assert_eq!(read.container, Container::Mov);
    assert_eq!(read.video.unwrap().codec, VideoCodec::H264);
    assert_eq!(read.subtitles.sidecar, Some(SidecarFormat::Srt));

    // A container that cannot carry what was read with it starts over.
    let mut torn = serde_json::to_value(ExportSettings::default()).unwrap();
    torn["container"] = serde_json::json!("wav");
    assert_eq!(last_settings(Some(&torn)), ExportSettings::default());
}

#[test]
fn the_file_is_named_after_the_project_in_a_folder_that_is_there() {
    assert_eq!(file_stem("Summer: cut 2/3"), "Summer cut 23");
    assert_eq!(file_stem("  ..hidden. "), "hidden");
    assert_eq!(file_stem("\u{7}?*"), UNTITLED);
    assert_eq!(file_stem(""), UNTITLED);
    assert_eq!(file_stem(&"a".repeat(500)).chars().count(), 120);

    let there = |folders: &'static [&'static str]| {
        move |folder: &Path| folders.iter().any(|f| Path::new(f) == folder)
    };
    let last = Path::new("/films/out");
    let project = Path::new("/work/Film.aster");
    let videos = Some(PathBuf::from("/home/me/Videos"));
    let home = Some(PathBuf::from("/home/me"));
    let all = there(&["/films/out", "/work", "/home/me/Videos", "/home/me"]);
    let chosen = initial_folder(Some(last), Some(project), videos.clone(), home.clone(), all);
    assert_eq!(chosen.as_deref(), Some(last));
    let gone = there(&["/work", "/home/me/Videos", "/home/me"]);
    let chosen = initial_folder(
        Some(last),
        Some(project),
        videos.clone(),
        home.clone(),
        gone,
    );
    assert_eq!(chosen.as_deref(), Some(Path::new("/work")));
    let unsaved = there(&["/home/me/Videos", "/home/me"]);
    let chosen = initial_folder(None, None, videos.clone(), home.clone(), unsaved);
    assert_eq!(chosen, videos);
    let bare = there(&["/home/me"]);
    assert_eq!(initial_folder(None, None, videos, home.clone(), bare), home);
    // A relative folder is never suggested.
    let relative = Path::new("out");
    assert_eq!(
        initial_folder(Some(relative), None, None, None, |_| true),
        None
    );
}

#[test]
fn the_selection_spans_its_clips() {
    let rig = rig();
    let project = rig.editor.project();
    let clip = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .next()
        .unwrap();
    assert_eq!(
        selection_span(project, &[clip.id]),
        Some((Time::ZERO, clip.end()))
    );
    assert_eq!(selection_span(project, &[]), None);
    assert_eq!(selection_span(project, &[Uuid::new_v4()]), None);
}

#[test]
fn a_stabilized_clip_without_its_analysis_is_warned_about() {
    let rig = rig();
    let mut project = rig.editor.project().clone();
    assert!(warnings(&project, &ExportSettings::default(), |_| true).is_empty());
    let track = (project.tracks.iter_mut())
        .find(|track| !track.clips.is_empty())
        .unwrap();
    let clip = &mut Arc::make_mut(track).clips[0];
    clip.effects
        .push(editor_core::Effect::new(EffectKind::Stabilize));
    // The clip starts a second in, so the first half second holds none.
    clip.start = Time::from_seconds(1.0);
    let said = warnings(&project, &ExportSettings::default(), |_| true);
    assert_eq!(said.len(), 1);
    assert!(said[0].contains("source.mp4 is stabilized"), "{}", said[0]);
    assert!(warnings(&project, &ExportSettings::default(), |_| false).is_empty());
    // Outside the range, nothing.
    let before = ExportSettings {
        range: ExportRange::Span {
            start: Time::ZERO,
            end: Time::from_seconds(0.5),
        },
        ..ExportSettings::default()
    };
    assert!(warnings(&project, &before, |_| true).is_empty());
    // A hidden track is not drawn, so not warned about.
    let mut hidden = project.clone();
    for track in hidden.tracks.iter_mut() {
        Arc::make_mut(track).hidden = true;
    }
    assert!(warnings(&hidden, &ExportSettings::default(), |_| true).is_empty());
}

#[test]
fn export_and_add_to_queue_make_the_job_asked_for() {
    let project = Arc::new(Project::new("A"));
    let context = Uuid::new_v4();
    let destination = PathBuf::from("/films/A.mp4");
    let file = Some(PathBuf::from("/work/A.aster"));
    let now = new_job(
        Arc::clone(&project),
        context,
        9,
        ExportSettings::default(),
        destination.clone(),
        true,
        file.clone(),
    );
    assert!(
        !now.start_paused,
        "Export renders when the queue reaches it"
    );
    assert_eq!(now.destination, destination);
    assert_eq!(now.project_path, file);
    assert_eq!((now.context, now.revision), (context, 9));
    assert!(Arc::ptr_eq(&now.project, &project));
    let later = new_job(
        project,
        context,
        9,
        ExportSettings::default(),
        destination,
        false,
        None,
    );
    assert!(later.start_paused, "Add to queue waits, paused");
    assert_eq!(later.project_path, None);
}

#[test]
fn quitting_asks_about_rendering_first_then_unsaved_work() {
    use CloseQuestion::*;
    assert_eq!(close_question(false, false, false), None);
    assert_eq!(close_question(false, false, true), Unsaved);
    assert_eq!(close_question(true, false, false), Rendering);
    assert_eq!(close_question(true, false, true), Rendering);
    assert_eq!(close_question(true, true, true), Unsaved);
    assert_eq!(close_question(true, true, false), None);
}

#[test]
fn capabilities_are_pending_until_probed_then_listed_with_presets() {
    let released = Arc::new(Mutex::new(false));
    let gate = Arc::clone(&released);
    let rig = rig_with(Arc::new(move || {
        while !*gate.lock().unwrap() {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(caps())
    }));
    let pending = rig.answer(AgentOperation::ExportCapabilities);
    assert_eq!(code(&pending), codes::CAPABILITIES_PENDING);
    assert_eq!(
        code(&rig.queue_export("Early.mp4", false)),
        codes::CAPABILITIES_PENDING
    );
    *released.lock().unwrap() = true;
    until("the capabilities", || {
        matches!(rig.queue.capabilities(), CapabilityState::Ready(_))
    });
    let request = rig.request(AgentOperation::ExportCapabilities);
    let presets = || editor_core::export::builtin_presets();
    match render_result(&rig.queue, &rig.editor, &request, presets, None) {
        AgentResult::ExportCapabilities {
            capabilities,
            presets,
            settings_schema,
        } => {
            assert_eq!(*capabilities, caps());
            assert!(presets.iter().any(|preset| preset.id == "recommended"));
            assert!(settings_schema.is_object());
        }
        other => panic!("expected capabilities, got {other:?}"),
    }

    let failed = rig_with(Arc::new(|| Err("FFmpeg was not found".into())));
    until("the probe to fail", || {
        matches!(failed.queue.capabilities(), CapabilityState::Failed(_))
    });
    let said = failed.answer(AgentOperation::ExportCapabilities);
    assert_eq!(code(&said), codes::UNSUPPORTED_EXPORT);
}

#[test]
fn queue_export_guards_the_target_and_the_destination() {
    let rig = rig();
    // A stale revision is the mutation guard's answer.
    let mut request = rig.request(AgentOperation::QueueExport {
        settings: ExportSettings::default(),
        destination: rig.dir.join("Film.mp4"),
        overwrite: false,
    });
    request.expected_revision = Some(rig.editor.revision() + 1);
    let stale = render_result(&rig.queue, &rig.editor, &request, Vec::new, None);
    assert_eq!(code(&stale), codes::STALE_REVISION);

    // An existing file needs overwrite.
    fs::write(rig.dir.join("Taken.mp4"), b"old").unwrap();
    let exists = rig.queue_export("Taken.mp4", false);
    assert_eq!(code(&exists), codes::DESTINATION_EXISTS);
    assert!(rig.queue.jobs().is_empty(), "nothing was queued");

    // The extension is the container's: an enqueue refusal.
    let wrong = rig.queue_export("Film.mov", false);
    assert_eq!(code(&wrong), codes::INVALID_EXPORT);
    // A source of the project is never written over, overwrite or not.
    let source = rig.answer(AgentOperation::QueueExport {
        settings: ExportSettings::default(),
        destination: rig.dir.join("source.mp4"),
        overwrite: true,
    });
    assert_eq!(code(&source), codes::INVALID_EXPORT);
    // Settings that contradict themselves are the plan's refusal.
    let audio_only_video = ExportSettings {
        container: Container::Wav,
        ..ExportSettings::default()
    };
    let invalid = rig.answer(AgentOperation::QueueExport {
        settings: audio_only_video,
        destination: rig.dir.join("Film.wav"),
        overwrite: false,
    });
    assert_eq!(code(&invalid), codes::INVALID_EXPORT);

    // With overwrite, the job is queued and renders at once.
    let queued = job(rig.queue_export("Taken.mp4", true));
    assert_eq!(queued.destination, rig.dir.join("Taken.mp4"));
    assert_eq!(queued.revision, rig.editor.revision());
    assert_eq!(queued.context_id, rig.editor.instance_id());
    until("the job to render", || {
        rig.queue.job(queued.id).unwrap().state == RenderState::Rendering
    });
}

#[test]
fn an_empty_timeline_is_an_empty_export() {
    let rig = rig();
    let empty = Editor::new(Project::new("Empty")).unwrap();
    let request = AgentRequest {
        request_id: "r".into(),
        expected_revision: Some(empty.revision()),
        expected_project_id: Some(empty.project().id),
        expected_context_id: Some(empty.instance_id()),
        operation: AgentOperation::QueueExport {
            settings: ExportSettings::default(),
            destination: rig.dir.join("Empty.mp4"),
            overwrite: false,
        },
    };
    let said = render_result(&rig.queue, &empty, &request, Vec::new, None);
    assert_eq!(code(&said), codes::EMPTY_EXPORT);
}

#[test]
fn job_operations_answer_with_the_job_or_unknown_job() {
    let rig = rig();
    let first = job(rig.queue_export("First.mp4", false));
    until("the job to render", || {
        rig.queue.job(first.id).unwrap().state == RenderState::Rendering
    });
    match rig.answer(AgentOperation::RenderJobs) {
        AgentResult::RenderJobs { jobs } => assert_eq!(jobs.len(), 1),
        other => panic!("expected jobs, got {other:?}"),
    }

    let paused = job(rig.answer(AgentOperation::PauseRender { job: first.id }));
    assert_eq!(paused.id, first.id);
    until("the job to pause", || {
        rig.queue.job(first.id).unwrap().state == RenderState::Paused
    });
    let resumed = job(rig.answer(AgentOperation::ResumeRender { job: first.id }));
    assert_eq!(resumed.id, first.id);
    // A running job cannot be removed.
    let refused = rig.answer(AgentOperation::RemoveRender { job: first.id });
    assert_eq!(code(&refused), codes::UNKNOWN_JOB);

    let cancelled = job(rig.answer(AgentOperation::CancelRender { job: first.id }));
    assert_eq!(cancelled.id, first.id);
    until("the job to stop", || {
        rig.queue.job(first.id).unwrap().state == RenderState::Cancelled
    });
    // A finished job cannot be paused, and says why.
    match rig.answer(AgentOperation::PauseRender { job: first.id }) {
        AgentResult::Error { code, message } => {
            assert_eq!(code, codes::UNKNOWN_JOB);
            assert!(message.contains("cancelled"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let retried = job(rig.answer(AgentOperation::RetryRender { job: first.id }));
    assert_eq!(retried.id, first.id);
    rig.finish.store(true, Ordering::SeqCst);
    until("the job to finish", || {
        rig.queue.job(first.id).unwrap().state == RenderState::Done
    });
    let removed = job(rig.answer(AgentOperation::RemoveRender { job: first.id }));
    assert_eq!(removed.state, RenderState::Done, "answered as it was");
    assert!(rig.queue.job(first.id).is_none());

    let nobody = Uuid::new_v4();
    for operation in [
        AgentOperation::PauseRender { job: nobody },
        AgentOperation::ResumeRender { job: nobody },
        AgentOperation::CancelRender { job: nobody },
        AgentOperation::RetryRender { job: nobody },
        AgentOperation::RemoveRender { job: nobody },
    ] {
        let said = rig.answer(operation);
        assert_eq!(code(&said), codes::UNKNOWN_JOB);
    }
}

#[test]
fn a_full_queue_says_so() {
    let rig = rig();
    for n in 0..crate::export::queue::MAX_JOBS {
        job(rig.queue_export(&format!("Job {n}.mp4"), false));
    }
    let full = rig.queue_export("One more.mp4", false);
    assert_eq!(code(&full), codes::QUEUE_FULL);
}

#[test]
fn the_file_manager_is_asked_for_the_files_folder() {
    let (program, args) = reveal_command(Path::new("/films/out/Film.mp4"));
    if cfg!(target_os = "linux") {
        assert_eq!(program, "xdg-open");
        assert_eq!(args, vec![std::ffi::OsString::from("/films/out")]);
    } else {
        assert!(!program.is_empty() && !args.is_empty());
    }
}

#[test]
fn explorer_is_given_the_switch_and_the_quoted_path_as_it_reads_them() {
    // Explorer takes `/select,"path"`; quoted whole, as an argument with a
    // space usually is, it opens Documents instead.
    let file = Path::new(r"C:\Users\Jane Doe\Videos\Film, take 2.mp4");
    assert_eq!(
        explorer_select(file),
        std::ffi::OsString::from(r#"/select,"C:\Users\Jane Doe\Videos\Film, take 2.mp4""#)
    );
    if cfg!(target_os = "windows") {
        let (program, args) = reveal_command(file);
        assert_eq!(program, "explorer");
        assert_eq!(args, vec![explorer_select(file)]);
    }
}

/// A rig whose project has an English caption track.
fn captioned_rig() -> Rig {
    let mut rig = rig();
    let mut project = rig.editor.project().clone();
    let mut track = editor_core::Track::new("Captions", TrackKind::Text);
    track.language = Some("en".into());
    track
        .clips
        .push(Clip::text("Hello", Time::ZERO, Time::from_seconds(1.0)));
    project.tracks.push(Arc::new(track));
    rig.editor = Editor::new(project).expect("FIXTURE: the project is valid");
    rig
}

fn with_srt() -> ExportSettings {
    let mut settings = ExportSettings::default();
    settings.subtitles.sidecar = Some(SidecarFormat::Srt);
    settings
}

#[test]
fn a_caption_file_there_is_replaced_only_when_asked() {
    let rig = captioned_rig();
    let destination = rig.dir.join("Talk.mp4");
    let caption = rig.dir.join("Talk.en.srt");
    let project = rig.editor.project();
    // The person is asked about each file there.
    assert!(replaced_files(project, &with_srt(), &destination, file_there).is_empty());
    fs::write(&caption, b"1\n00:00:00,000 --> 00:00:01,000\nHand made\n").unwrap();
    assert_eq!(
        replaced_files(project, &with_srt(), &destination, file_there),
        std::slice::from_ref(&caption)
    );
    assert!(
        replaced_files(
            project,
            &ExportSettings::default(),
            &destination,
            file_there
        )
        .is_empty(),
        "no caption file asked for"
    );
    fs::write(&destination, b"old").unwrap();
    assert_eq!(
        replaced_files(project, &with_srt(), &destination, file_there),
        [destination.clone(), caption.clone()]
    );
    fs::remove_file(&destination).unwrap();

    // An agent needs overwrite for it as for the file itself.
    let queue = |overwrite: bool| {
        rig.answer(AgentOperation::QueueExport {
            settings: with_srt(),
            destination: destination.clone(),
            overwrite,
        })
    };
    let refused = queue(false);
    assert_eq!(code(&refused), codes::DESTINATION_EXISTS);
    match &refused {
        AgentResult::Error { message, .. } => assert!(message.contains("Talk.en.srt"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(rig.queue.jobs().is_empty(), "nothing was queued");
    let queued = job(queue(true));
    assert_eq!(queued.destination, destination);
}

#[test]
fn an_agent_cannot_queue_a_file_another_job_writes() {
    let rig = rig();
    let first = job(rig.queue_export("Film.mp4", false));
    let again = rig.queue_export("Film.mp4", false);
    assert_eq!(code(&again), codes::INVALID_EXPORT);
    match again {
        AgentResult::Error { message, .. } => assert_eq!(
            message,
            "Another export in the queue already writes Film.mp4; choose another name"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(rig.queue.jobs().len(), 1);
    assert!(rig.queue.cancel(first.id));
}

#[test]
fn stop_and_quit_cancels_every_job_at_once_and_starts_none() {
    let rig = rig();
    let rendering = job(rig.queue_export("A.mp4", false)).id;
    until("A to render", || {
        rig.queue.job(rendering).map(|job| job.state) == Some(RenderState::Rendering)
    });
    let waiting = job(rig.queue_export("B.mp4", false)).id;
    let held = rig
        .queue
        .enqueue(new_job(
            rig.editor.snapshot(),
            rig.editor.instance_id(),
            rig.editor.revision(),
            ExportSettings::default(),
            rig.dir.join("C.mp4"),
            false,
            None,
        ))
        .expect("FIXTURE: C is queued held");
    assert!(rig.queue.busy(), "FIXTURE: quitting would ask");

    assert_eq!(stop_all(&rig.queue), 3);
    // At once: not when the quit goes through, and the job rendering
    // never gets to finish.
    rig.finish.store(true, Ordering::SeqCst);
    until("every job to stop", || !rig.queue.busy());
    for id in [rendering, waiting, held] {
        let stopped = rig.queue.job(id).expect("the job stays listed");
        assert_eq!(stopped.state, RenderState::Cancelled, "{}", stopped.name);
    }
    for id in [waiting, held] {
        assert_eq!(
            rig.queue.job(id).unwrap().attempts,
            0,
            "a waiting job is not started as the one rendering stops"
        );
    }
    assert!(!rig.dir.join("A.mp4").exists(), "no file was made");
    // Should the person stay, they can be rendered again.
    rig.finish.store(false, Ordering::SeqCst);
    assert!(rig.queue.retry(waiting));
    assert_eq!(stop_all(&rig.queue), 1);
    assert_eq!(stop_all(&rig.queue), 0, "nothing is left to stop");
}

#[test]
fn a_finished_job_is_told_by_a_notice_unless_the_queue_shows_it() {
    let event = |result: Result<PathBuf, String>, note: Option<&str>| RenderEvent::Finished {
        id: Uuid::new_v4(),
        name: "Trip.mp4".into(),
        result,
        output_bytes: Some(1),
        note: note.map(str::to_owned),
    };
    let made = event(Ok("/films/Trip.mp4".into()), None);
    let noted = event(Ok("/films/Trip.mp4".into()), Some("rendered in software"));
    let failed = event(Err("FFmpeg stopped".into()), None);
    assert_eq!(
        finished_notice(&made, false),
        Some(("Exported Trip.mp4".to_owned(), false))
    );
    assert_eq!(
        finished_notice(&noted, false),
        Some(("Exported Trip.mp4: rendered in software".to_owned(), false))
    );
    assert_eq!(
        finished_notice(&failed, false),
        Some(("Trip.mp4 was not exported: FFmpeg stopped".to_owned(), true))
    );
    for event in [made, noted, failed] {
        assert_eq!(finished_notice(&event, true), None, "the row says it");
    }
}
