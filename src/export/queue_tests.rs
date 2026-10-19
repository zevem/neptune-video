//! The render queue's rules with a fake job runner: no FFmpeg, no device.
//!
//! The fake draws ten "frames", each taking a permit the test hands out, so a
//! test decides exactly how far a job gets. It also honours pause between
//! frames and cancel while it waits, as the engine does.
use super::*;
use crate::export::engine::{self, Finish, part_path, temp_path};
use editor_core::{
    Asset, Clip, FrameRate, MediaKind, Time, TrackKind,
    export::{AudioEncoder, Container, ExportRange, VideoEncoder, known_video_encoder},
};
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

const FRAMES: u64 = 10;
const PATIENCE: Duration = Duration::from_secs(5);

/// A folder of the test's own, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("neptune-video-queue-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("FIXTURE: the scratch folder is made");
        Self(
            path.canonicalize()
                .expect("FIXTURE: the scratch folder resolves"),
        )
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

/// Frames the fake may draw; each frame takes one.
#[derive(Default)]
struct Permits {
    left: Mutex<u64>,
    changed: Condvar,
}

impl Permits {
    fn give(&self, count: u64) {
        let mut left = self.left.lock().unwrap();
        *left = left.saturating_add(count);
        self.changed.notify_all();
    }

    fn open(&self) {
        self.give(u64::MAX / 2);
    }

    /// Takes one; `false` when the job was cancelled while it waited.
    fn take(&self, handle: &Handle) -> bool {
        let mut left = self.left.lock().unwrap();
        loop {
            if handle.cancelled() {
                return false;
            }
            if *left > 0 {
                *left -= 1;
                return true;
            }
            left = (self.changed)
                .wait_timeout(left, Duration::from_millis(2))
                .unwrap()
                .0;
        }
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

struct Rig {
    queue: RenderQueue,
    wakes: Arc<AtomicUsize>,
    started: mpsc::Receiver<Uuid>,
    permits: Arc<Permits>,
    /// Cancellers the fake registered that were called.
    killed: Arc<AtomicUsize>,
    project: Arc<Project>,
    /// The source media's path.
    source: PathBuf,
    dir: Scratch,
}

fn rig() -> Rig {
    let rig = rig_with(Arc::new(|| Ok(caps())));
    until("the capabilities", || {
        matches!(rig.queue.capabilities(), CapabilityState::Ready(_))
    });
    rig
}

fn rig_with(probe: Probe) -> Rig {
    let dir = Scratch::new("rig");
    let source = dir.join("source.mp4");
    fs::write(&source, b"not really a video").unwrap();
    let mut asset = Asset::new(source.clone(), MediaKind::Video, Time::from_seconds(2.0));
    asset.width = 320;
    asset.height = 180;
    asset.frame_rate = Some(FrameRate::default());
    let mut project = Project::new("Queue test");
    project.settings.width = 320;
    project.settings.height = 180;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let track = (project.tracks.iter_mut())
        .find(|track| track.kind == TrackKind::Video)
        .expect("a new project has a video track");
    Arc::make_mut(track)
        .clips
        .push(Clip::from_asset(&asset, Time::ZERO));
    let wakes = Arc::new(AtomicUsize::new(0));
    let (tell, started) = mpsc::channel();
    let permits = Arc::new(Permits::default());
    let killed = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&wakes);
    let queue = RenderQueue::with_parts(
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
        },
        probe,
        fake(tell, Arc::clone(&permits), Arc::clone(&killed)),
    );
    Rig {
        queue,
        wakes,
        started,
        permits,
        killed,
        project: Arc::new(project),
        source,
        dir,
    }
}

/// The fake job: ten frames of permits; a name starting `fail` fails at the
/// end, one starting `panic` leaves files behind and panics.
fn fake(started: mpsc::Sender<Uuid>, permits: Arc<Permits>, killed: Arc<AtomicUsize>) -> Runner {
    Box::new(move |ticket, handle| {
        let _ = started.send(ticket.id);
        let killed = Arc::clone(&killed);
        handle.on_cancel(move || {
            killed.fetch_add(1, Ordering::SeqCst);
        });
        let stem = (ticket.destination.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if stem.starts_with("panic") {
            fs::write(part_path(&ticket.destination, ticket.id), b"part").unwrap();
            fs::create_dir_all(temp_path(ticket.id)).unwrap();
            panic!("the fake runner broke");
        }
        handle.phase(RenderState::Rendering);
        for frame in 0..FRAMES {
            if !permits.take(handle) || handle.hold().is_none() {
                return Finish::Cancelled;
            }
            handle.report(Report {
                progress: (frame + 1) as f32 / FRAMES as f32,
                frames_done: frame + 1,
                frames_total: FRAMES,
                ..Report::default()
            });
        }
        if stem.starts_with("fail") {
            return Finish::Failed("The encoder broke".into());
        }
        Finish::Done {
            path: ticket.destination.clone(),
            bytes: Some(42),
            note: None,
        }
    })
}

impl Rig {
    fn job(&self, name: &str) -> NewJob {
        NewJob {
            project: Arc::clone(&self.project),
            context: Uuid::nil(),
            revision: 7,
            settings: ExportSettings::default(),
            destination: self.dir.join(name),
            start_paused: false,
            project_path: None,
        }
    }

    fn add(&self, name: &str) -> Uuid {
        self.queue
            .enqueue(self.job(name))
            .unwrap_or_else(|error| panic!("FIXTURE: {name} is queued: {error}"))
    }

    fn info(&self, id: Uuid) -> RenderJobInfo {
        self.queue.job(id).expect("the job is listed")
    }

    fn state(&self, id: Uuid) -> RenderState {
        self.info(id).state
    }

    fn next_started(&self) -> Uuid {
        self.started.recv_timeout(PATIENCE).expect("a job starts")
    }

    fn none_started(&self) {
        assert!(
            self.started
                .recv_timeout(Duration::from_millis(80))
                .is_err(),
            "no other job starts"
        );
    }

    fn wait_state(&self, id: Uuid, state: RenderState) {
        until(&format!("{state:?}"), || self.state(id) == state);
    }

    fn names(&self) -> Vec<String> {
        self.queue.jobs().into_iter().map(|job| job.name).collect()
    }
}

#[track_caller]
fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn jobs_render_one_at_a_time_in_the_order_listed() {
    let rig = rig();
    let a = rig.add("a.mp4");
    let b = rig.add("b.mp4");
    let c = rig.add("c.mp4");
    assert_eq!(rig.next_started(), a);
    rig.none_started();
    let summary = rig.queue.summary();
    assert_eq!(summary.active, 1);
    assert_eq!(summary.queued, 2);
    assert_eq!(summary.rendering, Some(a));
    assert!(!summary.paused);
    assert_eq!(rig.state(a), RenderState::Rendering);
    assert_eq!(rig.state(b), RenderState::Queued);
    assert!(rig.queue.busy());

    rig.permits.open();
    assert_eq!(rig.next_started(), b);
    assert_eq!(rig.next_started(), c);
    rig.wait_state(c, RenderState::Done);
    for id in [a, b, c] {
        let info = rig.info(id);
        assert_eq!(info.state, RenderState::Done);
        assert_eq!(info.progress, 1.0);
        assert_eq!(info.frames_done, FRAMES);
        assert_eq!(info.output_bytes, Some(42));
        assert_eq!(info.attempts, 1);
        assert_eq!(info.revision, 7);
        assert_eq!(info.encoder.as_deref(), Some("libx264"));
    }
    let finished: Vec<Uuid> = std::iter::from_fn(|| rig.queue.poll())
        .map(|RenderEvent::Finished { id, result, .. }| {
            assert!(result.is_ok());
            id
        })
        .collect();
    assert_eq!(finished, [a, b, c], "each job is told of once, in order");
    assert!(!rig.queue.busy());
    assert_eq!(rig.queue.summary(), QueueSummary::default());
}

#[test]
fn a_paused_job_stops_between_frames_and_holds_the_queue() {
    let rig = rig();
    let a = rig.add("a.mp4");
    assert_eq!(rig.next_started(), a);
    rig.permits.give(2);
    until("two frames", || rig.info(a).frames_done == 2);
    assert!(rig.queue.pause(a));
    assert!(!rig.queue.pause(a), "a paused job is not paused again");
    assert_eq!(rig.state(a), RenderState::Paused);
    let b = rig.add("b.mp4");
    rig.permits.give(5);
    thread::sleep(Duration::from_millis(150));
    let held = rig.info(a);
    assert_eq!(held.frames_done, 2, "no frame is drawn while paused");
    assert_eq!(held.state, RenderState::Paused);
    rig.none_started();
    let summary = rig.queue.summary();
    assert!(summary.paused);
    assert_eq!(summary.rendering, Some(a));
    assert_eq!(summary.queued, 1);

    assert!(rig.queue.resume(a));
    assert!(!rig.queue.resume(a));
    rig.permits.open();
    rig.wait_state(a, RenderState::Done);
    assert_eq!(rig.next_started(), b);
    rig.wait_state(b, RenderState::Done);
    assert_eq!(rig.info(a).frames_done, FRAMES);
}

#[test]
fn a_paused_waiting_job_is_skipped_until_resumed() {
    let rig = rig();
    let a = rig
        .queue
        .enqueue(NewJob {
            start_paused: true,
            ..rig.job("a.mp4")
        })
        .unwrap();
    assert_eq!(rig.state(a), RenderState::Paused);
    let b = rig.add("b.mp4");
    assert_eq!(rig.next_started(), b, "the paused job is passed over");
    let c = rig.add("c.mp4");
    assert!(rig.queue.pause(c));
    assert!(rig.queue.resume(a));
    assert_eq!(rig.state(a), RenderState::Queued);
    rig.permits.open();
    assert_eq!(rig.next_started(), a);
    rig.wait_state(a, RenderState::Done);
    rig.none_started();
    assert_eq!(rig.state(c), RenderState::Paused);
    assert!(
        rig.queue.busy(),
        "a paused waiting job keeps the queue busy"
    );
    assert_eq!(rig.queue.summary().queued, 0);
    assert_eq!(rig.queue.summary().held, 1);
    assert!(rig.queue.resume(c));
    assert_eq!(rig.next_started(), c);
    rig.wait_state(c, RenderState::Done);
}

#[test]
fn hold_reports_no_wait_unless_it_was_paused() {
    // The engine clears its speed window whenever `hold` waited; a nonzero
    // answer for an unpaused job left every job at 0 fps with no time left.
    let control = Arc::new(Control::new(false));
    for _ in 0..3 {
        assert_eq!(control.hold(), Some(Duration::ZERO));
    }
    control.set_paused(true);
    let resumer = {
        let control = Arc::clone(&control);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            control.set_paused(false);
        })
    };
    let waited = control.hold().expect("a resumed job goes on");
    resumer.join().unwrap();
    assert!(waited >= Duration::from_millis(20), "waited {waited:?}");
    assert_eq!(control.hold(), Some(Duration::ZERO));
    control.cancel();
    assert_eq!(control.hold(), None);
}

#[test]
fn cancel_stops_a_waiting_job_at_once_and_kills_the_running_one() {
    let rig = rig();
    let a = rig.add("a.mp4");
    let b = rig.add("b.mp4");
    let c = rig.add("c.mp4");
    assert_eq!(rig.next_started(), a);
    assert!(rig.queue.cancel(b));
    assert_eq!(rig.state(b), RenderState::Cancelled);
    assert!(!rig.queue.cancel(b), "a cancelled job is finished");
    rig.permits.give(1);
    until("a frame", || rig.info(a).frames_done == 1);
    assert!(rig.queue.cancel(a));
    rig.wait_state(a, RenderState::Cancelled);
    assert_eq!(rig.killed.load(Ordering::SeqCst), 1, "its canceller ran");
    assert_eq!(rig.next_started(), c, "the cancelled waiting job never ran");
    // Cancelling a paused running job wakes it to stop.
    rig.permits.give(1);
    until("a frame of c", || rig.info(c).frames_done == 1);
    assert!(rig.queue.pause(c));
    rig.permits.give(1);
    thread::sleep(Duration::from_millis(30));
    assert!(rig.queue.cancel(c));
    rig.wait_state(c, RenderState::Cancelled);
    assert_eq!(rig.queue.poll(), None, "a cancel is not announced");
    assert!(!rig.queue.busy());
    assert!(!rig.queue.cancel(Uuid::new_v4()));
}

#[test]
fn retry_runs_a_failed_or_cancelled_job_again_from_its_snapshot() {
    let rig = rig();
    rig.permits.open();
    let failing = rig.add("fail.mp4");
    rig.wait_state(failing, RenderState::Failed);
    let failed = rig.info(failing);
    assert_eq!(failed.message.as_deref(), Some("The encoder broke"));
    assert_eq!(failed.attempts, 1);
    match rig.queue.poll() {
        Some(RenderEvent::Finished { id, result, .. }) => {
            assert_eq!(id, failing);
            assert_eq!(result, Err("The encoder broke".into()));
        }
        other => panic!("expected the failure, got {other:?}"),
    }
    assert!(rig.queue.retry(failing));
    assert!(!rig.queue.retry(failing), "a waiting job is not retried");
    rig.wait_state(failing, RenderState::Failed);
    assert_eq!(rig.info(failing).attempts, 2, "each run is counted");

    let done = rig.add("done.mp4");
    rig.wait_state(done, RenderState::Done);
    assert!(!rig.queue.retry(done), "a finished file is not made again");
    assert!(!rig.queue.retry(Uuid::new_v4()));
}

#[test]
fn a_cancelled_job_retried_renders_again() {
    let rig = rig();
    let a = rig.add("a.mp4");
    assert_eq!(rig.next_started(), a);
    rig.permits.give(3);
    until("three frames", || rig.info(a).frames_done == 3);
    assert!(rig.queue.cancel(a));
    rig.wait_state(a, RenderState::Cancelled);
    assert!(rig.queue.retry(a));
    let retried = rig.info(a);
    assert_eq!(retried.state, RenderState::Queued);
    assert_eq!(retried.frames_done, 0, "progress starts over");
    assert_eq!(rig.next_started(), a);
    rig.permits.open();
    rig.wait_state(a, RenderState::Done);
    assert_eq!(rig.info(a).attempts, 2);
}

#[test]
fn only_finished_or_waiting_jobs_are_removed() {
    let rig = rig();
    let a = rig.add("a.mp4");
    let b = rig.add("b.mp4");
    assert_eq!(rig.next_started(), a);
    assert!(!rig.queue.remove(a), "a running job is cancelled first");
    assert!(rig.queue.pause(a));
    assert!(
        !rig.queue.remove(a),
        "a paused running job is cancelled first"
    );
    assert!(rig.queue.remove(b));
    assert!(!rig.queue.remove(b));
    assert!(rig.queue.resume(a));
    rig.permits.open();
    rig.wait_state(a, RenderState::Done);
    assert!(rig.queue.remove(a));
    assert!(rig.queue.jobs().is_empty());
    rig.none_started();
}

#[test]
fn waiting_jobs_move_among_themselves() {
    let rig = rig();
    let a = rig.add("a.mp4");
    let b = rig.add("b.mp4");
    let c = rig.add("c.mp4");
    let d = rig.add("d.mp4");
    assert_eq!(rig.next_started(), a);
    assert!(rig.queue.move_by(d, -2));
    assert_eq!(rig.names(), ["a.mp4", "d.mp4", "b.mp4", "c.mp4"]);
    assert!(
        !rig.queue.move_by(d, -1),
        "the first waiting job cannot go sooner"
    );
    assert!(!rig.queue.move_by(b, 5));
    assert!(!rig.queue.move_by(b, 0));
    assert!(!rig.queue.move_by(a, 1), "a running job does not move");
    assert!(rig.queue.move_by(d, 2));
    assert_eq!(rig.names(), ["a.mp4", "b.mp4", "c.mp4", "d.mp4"]);
    assert!(rig.queue.move_by(c, -1));
    rig.permits.open();
    assert_eq!(rig.next_started(), c);
    assert_eq!(rig.next_started(), b);
    assert_eq!(rig.next_started(), d);
    rig.wait_state(d, RenderState::Done);
}

#[test]
fn clear_finished_keeps_what_is_left_to_do() {
    let rig = rig();
    rig.permits.open();
    let done = rig.add("done.mp4");
    let failed = rig.add("fail.mp4");
    rig.wait_state(failed, RenderState::Failed);
    rig.wait_state(done, RenderState::Done);
    let waiting = rig
        .queue
        .enqueue(NewJob {
            start_paused: true,
            ..rig.job("waiting.mp4")
        })
        .unwrap();
    rig.queue.clear_finished();
    let left: Vec<Uuid> = rig.queue.jobs().into_iter().map(|job| job.id).collect();
    assert_eq!(left, [waiting]);
}

#[test]
fn at_most_sixty_four_jobs_are_listed() {
    let rig = rig();
    for index in 0..MAX_JOBS {
        rig.queue
            .enqueue(NewJob {
                start_paused: true,
                ..rig.job(&format!("{index}.mp4"))
            })
            .unwrap();
    }
    assert_eq!(
        rig.queue.enqueue(rig.job("one-more.mp4")),
        Err(EnqueueError::QueueFull)
    );
    assert_eq!(EnqueueError::QueueFull.code(), "queue_full");
    rig.queue.clear_finished();
    assert_eq!(rig.queue.jobs().len(), MAX_JOBS, "waiting jobs are kept");
    rig.none_started();
}

impl Rig {
    /// Queues `job`, which the queue takes, and waits for it to fail as it
    /// starts, before the runner sees it. Says why it failed.
    #[track_caller]
    fn fails_at_start(&self, job: NewJob) -> String {
        let id = self
            .queue
            .enqueue(job)
            .unwrap_or_else(|error| panic!("the names pass: {error}"));
        self.wait_state(id, RenderState::Failed);
        assert!(
            self.started.try_recv().is_err(),
            "the runner never saw the job"
        );
        let message = self.info(id).message.unwrap_or_default();
        let _ = self.queue.poll();
        assert!(self.queue.remove(id));
        message
    }
}

#[test]
fn a_destination_must_be_a_new_file_the_project_does_not_own() {
    let rig = rig();
    rig.permits.open();
    let refused = |job: NewJob| {
        rig.queue
            .enqueue(job)
            .expect_err("the destination is refused")
    };
    assert_eq!(
        refused(NewJob {
            destination: PathBuf::from("relative/out.mp4"),
            ..rig.job("x.mp4")
        }),
        EnqueueError::DestinationNotAbsolute
    );
    assert_eq!(
        refused(rig.job("out.mov")),
        EnqueueError::WrongExtension { expected: "mp4" }
    );
    assert_eq!(
        refused(rig.job("out")),
        EnqueueError::WrongExtension { expected: "mp4" }
    );

    // The source by name, at once: its path, with `..`, in another case.
    assert_eq!(
        refused(rig.job("source.mp4")),
        EnqueueError::DestinationIsSource
    );
    assert_eq!(
        refused(NewJob {
            destination: rig.dir.join("sub/../source.mp4"),
            ..rig.job("x.mp4")
        }),
        EnqueueError::DestinationIsSource
    );
    assert_eq!(
        refused(rig.job("SOURCE.MP4")),
        EnqueueError::DestinationIsSource,
        "a FAT card ignores case, and Linux does not fold it"
    );
    // A source that is missing is still compared by name.
    let mut project = (*rig.project).clone();
    let mut gone = Asset::new(
        rig.dir.join("gone.mp4"),
        MediaKind::Video,
        Time::from_seconds(1.0),
    );
    gone.width = 320;
    gone.height = 180;
    Arc::make_mut(&mut project.assets).insert(gone.id, gone);
    assert_eq!(
        refused(NewJob {
            project: Arc::new(project),
            ..rig.job("gone.mp4")
        }),
        EnqueueError::DestinationIsSource
    );

    // The project's file and what is kept beside it.
    let document = rig.dir.join("film.nvp");
    fs::write(&document, b"{}").unwrap();
    let checkpoints = rig.dir.join("film.nvp.checkpoints");
    fs::create_dir(&checkpoints).unwrap();
    let beside = |name: &str| NewJob {
        destination: rig.dir.join(name),
        settings: ExportSettings::default(),
        project_path: Some(document.clone()),
        ..rig.job("x.mp4")
    };
    for name in ["film.nvp.recovery", "film.nvp.lock", "film.nvp.write-lock"] {
        // Named with the container's extension they are other files; the
        // sidecar names themselves are refused whatever the extension says.
        let job = beside(name);
        let error = refused(job);
        assert!(
            matches!(
                error,
                EnqueueError::WrongExtension { .. } | EnqueueError::DestinationIsProject
            ),
            "{name}: {error:?}"
        );
    }
    assert_eq!(
        refused(beside("film.nvp.checkpoints/take.mp4")),
        EnqueueError::DestinationIsProject
    );
    assert_eq!(
        refused(beside("FILM.NVP.CHECKPOINTS/take.mp4")),
        EnqueueError::DestinationIsProject
    );
    // A project file that has the container's extension is itself refused.
    let odd = rig.dir.join("odd.mp4");
    fs::write(&odd, b"{}").unwrap();
    assert_eq!(
        refused(NewJob {
            destination: odd.clone(),
            project_path: Some(odd.clone()),
            ..rig.job("x.mp4")
        }),
        EnqueueError::DestinationIsProject
    );
    assert_eq!(EnqueueError::DestinationIsProject.code(), "invalid_export");

    // What names resolve to is looked at as the job starts, off the
    // window's thread, and fails it with the same reason.
    assert_eq!(
        rig.fails_at_start(NewJob {
            destination: rig.dir.join("missing/out.mp4"),
            ..rig.job("x.mp4")
        }),
        EnqueueError::FolderMissing.to_string()
    );
    fs::create_dir(rig.dir.join("folder.mp4")).unwrap();
    assert_eq!(
        rig.fails_at_start(rig.job("folder.mp4")),
        EnqueueError::DestinationIsFolder.to_string()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&rig.source, rig.dir.join("link.mp4")).unwrap();
        assert_eq!(
            rig.fails_at_start(rig.job("link.mp4")),
            EnqueueError::DestinationIsSource.to_string()
        );
        // A hard link is the source under another name.
        fs::hard_link(&rig.source, rig.dir.join("hard.mp4")).unwrap();
        assert_eq!(
            rig.fails_at_start(rig.job("hard.mp4")),
            EnqueueError::DestinationIsSource.to_string()
        );
        let linked = rig.dir.join("linked-project.mp4");
        fs::hard_link(&document, &linked).unwrap();
        assert_eq!(
            rig.fails_at_start(NewJob {
                destination: linked,
                project_path: Some(document.clone()),
                ..rig.job("x.mp4")
            }),
            EnqueueError::DestinationIsProject.to_string()
        );
        // A link to the folder resolves to the source inside it, in
        // another case too.
        std::os::unix::fs::symlink(&rig.dir.0, rig.dir.join("alias")).unwrap();
        assert_eq!(
            rig.fails_at_start(NewJob {
                destination: rig.dir.join("alias/Source.mp4"),
                ..rig.job("x.mp4")
            }),
            EnqueueError::DestinationIsSource.to_string()
        );
    }

    // An existing other file is the caller's to decide on.
    fs::write(rig.dir.join("existing.mp4"), b"old").unwrap();
    let existing = rig.add("existing.mp4");
    let fine = rig.add("fine.mp4");
    rig.wait_state(existing, RenderState::Done);
    rig.wait_state(fine, RenderState::Done);
}

/// A project with a caption track in `language`, for the caption file.
fn captioned(rig: &Rig, language: &str) -> Arc<Project> {
    let mut project = (*rig.project).clone();
    let mut track = editor_core::Track::new("Captions", TrackKind::Text);
    track.language = Some(language.into());
    track
        .clips
        .push(Clip::text("Hello", Time::ZERO, Time::from_seconds(1.0)));
    project.tracks.push(Arc::new(track));
    Arc::new(project)
}

#[test]
fn no_two_unfinished_jobs_write_the_same_file() {
    let rig = rig();
    let held = |name: &str| NewJob {
        start_paused: true,
        ..rig.job(name)
    };
    let first = rig.queue.enqueue(held("Film.mp4")).unwrap();
    // Another preset, the same file: the second would replace the first.
    let refused = rig
        .queue
        .enqueue(held("Film.mp4"))
        .expect_err("the file is taken");
    assert_eq!(refused, EnqueueError::DestinationQueued("Film.mp4".into()));
    assert_eq!(refused.code(), "invalid_export");
    assert_eq!(
        refused.to_string(),
        "Another export in the queue already writes Film.mp4; choose another name"
    );
    assert_eq!(
        rig.queue.enqueue(held("film.MP4")),
        Err(EnqueueError::DestinationQueued("film.MP4".into())),
        "names that differ in case alone are one file on many volumes"
    );
    assert_eq!(
        rig.queue.enqueue(NewJob {
            destination: rig.dir.join("sub/../Film.mp4"),
            ..held("x.mp4")
        }),
        Err(EnqueueError::DestinationQueued("Film.mp4".into()))
    );

    // A caption file one job writes is taken too.
    let srt = |name: &str| NewJob {
        project: captioned(&rig, "en"),
        settings: ExportSettings {
            subtitles: editor_core::export::SubtitleSettings {
                sidecar: Some(editor_core::export::SidecarFormat::Srt),
                ..Default::default()
            },
            ..ExportSettings::default()
        },
        ..held(name)
    };
    let talk = rig.queue.enqueue(srt("Talk.mp4")).unwrap();
    assert_eq!(
        rig.queue.enqueue(srt("Talk.mp4")),
        Err(EnqueueError::DestinationQueued("Talk.mp4".into()))
    );
    // A MOV of the same name writes another file but the same captions.
    let mov = NewJob {
        settings: ExportSettings {
            container: editor_core::export::Container::Mov,
            ..srt("x.mp4").settings
        },
        ..srt("Talk.mov")
    };
    assert_eq!(
        rig.queue.enqueue(mov),
        Err(EnqueueError::DestinationQueued("Talk.en.srt".into()))
    );

    // Once a job is finished its file is free again; a retry then asks.
    assert!(rig.queue.cancel(first));
    let second = rig.queue.enqueue(held("Film.mp4")).unwrap();
    assert!(
        !rig.queue.retry(first),
        "the retry would write what the second job writes"
    );
    assert!(rig.queue.cancel(second));
    assert!(rig.queue.retry(first));
    assert!(rig.queue.cancel(talk));
    assert_eq!(rig.queue.jobs().len(), 3);
}

#[test]
fn held_jobs_are_counted_apart_and_resumed_together() {
    let rig = rig();
    let held = |name: &str| {
        rig.queue
            .enqueue(NewJob {
                start_paused: true,
                ..rig.job(name)
            })
            .unwrap()
    };
    let a = rig.add("a.mp4");
    assert_eq!(rig.next_started(), a);
    let b = held("b.mp4");
    let c = rig.add("c.mp4");
    let d = held("d.mp4");
    let summary = rig.queue.summary();
    assert_eq!((summary.active, summary.queued, summary.held), (1, 1, 2));
    assert_eq!(rig.queue.waiting(), [b, c, d], "held jobs wait too");
    // Held jobs move among the waiting ones as listed.
    assert!(rig.queue.move_by(d, -2));
    assert_eq!(rig.queue.waiting(), [d, b, c]);
    // The started job, paused, is not held: Start all leaves it paused.
    assert!(rig.queue.pause(a));
    assert_eq!(rig.queue.resume_all(), 2);
    assert_eq!(rig.state(a), RenderState::Paused);
    assert_eq!(rig.state(b), RenderState::Queued);
    assert_eq!(rig.state(d), RenderState::Queued);
    let summary = rig.queue.summary();
    assert_eq!((summary.queued, summary.held), (3, 0));
    assert!(summary.paused);
    assert_eq!(rig.queue.resume_all(), 0);
    assert!(rig.queue.resume(a));
    rig.permits.open();
    assert_eq!(rig.next_started(), d);
    assert_eq!(rig.next_started(), b);
    assert_eq!(rig.next_started(), c);
    rig.wait_state(c, RenderState::Done);
}

#[test]
fn a_starting_job_removes_what_stopped_exports_left_in_its_folder() {
    let rig = rig();
    rig.permits.open();
    let hour_ago = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    let age = |path: &Path| {
        let file = fs::File::open(path).unwrap();
        file.set_modified(hour_ago).unwrap();
    };
    // A part file and a working folder of an export that stopped long ago.
    let stale_part = rig.dir.join(&format!(".Old.mp4.{}.part", Uuid::new_v4()));
    fs::write(&stale_part, b"half a film").unwrap();
    age(&stale_part);
    let stale_work = rig
        .dir
        .join(&format!(".neptune-video-render-{}", Uuid::new_v4()));
    fs::create_dir(&stale_work).unwrap();
    let sound = stale_work.join("sound.f32");
    fs::write(&sound, b"sound").unwrap();
    age(&sound);
    age(&stale_work);
    // One still being written, and files that are not an export's.
    let fresh = rig.dir.join(&format!(".New.mp4.{}.part", Uuid::new_v4()));
    fs::write(&fresh, b"being written").unwrap();
    let theirs = rig.dir.join(".notes.part");
    fs::write(&theirs, b"someone else's").unwrap();
    age(&theirs);
    // A job listed, held, keeps its part however old.
    let held = rig
        .queue
        .enqueue(NewJob {
            start_paused: true,
            ..rig.job("held.mp4")
        })
        .unwrap();
    let held_part = part_path(&rig.dir.join("held.mp4"), held);
    fs::write(&held_part, b"held").unwrap();
    age(&held_part);

    let a = rig.add("a.mp4");
    rig.wait_state(a, RenderState::Done);
    assert!(!stale_part.exists(), "the old part file went");
    assert!(!stale_work.exists(), "the old working folder went");
    assert!(fresh.exists(), "a recent part file stays");
    assert!(theirs.exists(), "a file without a job's id stays");
    assert!(held_part.exists(), "a listed job's part stays");
}

#[test]
fn stale_working_folders_in_the_temporary_folder_are_removed() {
    let dir = Scratch::new("temp-sweep");
    let hour_ago = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    let old = dir.join(&format!("neptune-video-render-{}", Uuid::new_v4()));
    fs::create_dir(&old).unwrap();
    fs::File::open(&old)
        .unwrap()
        .set_modified(hour_ago)
        .unwrap();
    let recent = dir.join(&format!("neptune-video-render-{}", Uuid::new_v4()));
    fs::create_dir(&recent).unwrap();
    let other = dir.join("neptune-video-render-notes");
    fs::create_dir(&other).unwrap();
    fs::File::open(&other)
        .unwrap()
        .set_modified(hour_ago)
        .unwrap();
    assert_eq!(engine::sweep_temp(&dir.0, SystemTime::now()), 1);
    assert!(!old.exists());
    assert!(recent.exists());
    assert!(other.exists());
}

#[test]
fn settings_are_checked_against_the_project_when_queued() {
    let rig = rig();
    let empty = Arc::new(Project::new("Empty"));
    let error = rig
        .queue
        .enqueue(NewJob {
            project: empty,
            ..rig.job("empty.mp4")
        })
        .expect_err("an empty timeline is refused");
    assert!(matches!(
        error,
        EnqueueError::Invalid(ExportError::Empty(_))
    ));
    assert_eq!(error.code(), "empty_export");
    let error = rig
        .queue
        .enqueue(NewJob {
            settings: ExportSettings {
                range: ExportRange::Span {
                    start: Time::from_seconds(5.0),
                    end: Time::from_seconds(6.0),
                },
                ..ExportSettings::default()
            },
            ..rig.job("late.mp4")
        })
        .expect_err("a range past the end is refused");
    assert_eq!(error.code(), "empty_export");
    let mut hevc = ExportSettings::default();
    hevc.video.as_mut().unwrap().codec = editor_core::export::VideoCodec::H265;
    let error = rig
        .queue
        .enqueue(NewJob {
            settings: hevc,
            ..rig.job("hevc.mp4")
        })
        .expect_err("a codec this host lacks is refused");
    assert_eq!(error.code(), "unsupported_export");
    assert!(rig.queue.jobs().is_empty());
}

#[test]
fn nothing_is_queued_before_the_capabilities_are_known() {
    let (release, released) = mpsc::channel::<Result<Capabilities, String>>();
    let released = Mutex::new(released);
    let rig = rig_with(Arc::new(move || {
        released
            .lock()
            .unwrap()
            .recv()
            .unwrap_or_else(|_| Err("the test ended".into()))
    }));
    assert_eq!(rig.queue.capabilities(), CapabilityState::Probing);
    let error = rig.queue.enqueue(rig.job("a.mp4")).unwrap_err();
    assert_eq!(error, EnqueueError::CapabilitiesPending);
    assert_eq!(error.code(), "capabilities_pending");

    let wakes = rig.wakes.load(Ordering::SeqCst);
    release.send(Err("ffmpeg was not found".into())).unwrap();
    until("the failure", || {
        matches!(rig.queue.capabilities(), CapabilityState::Failed(_))
    });
    until("a wake", || rig.wakes.load(Ordering::SeqCst) > wakes);
    assert_eq!(
        rig.queue.enqueue(rig.job("a.mp4")),
        Err(EnqueueError::CapabilitiesFailed(
            "ffmpeg was not found".into()
        ))
    );

    rig.queue.reprobe();
    assert_eq!(rig.queue.capabilities(), CapabilityState::Probing);
    release.send(Ok(caps())).unwrap();
    until("the capabilities", || {
        matches!(rig.queue.capabilities(), CapabilityState::Ready(_))
    });
    let a = rig.add("a.mp4");
    assert_eq!(rig.next_started(), a);
    // A probe running again holds waiting jobs, not the running one.
    let b = rig.add("b.mp4");
    rig.queue.reprobe();
    rig.permits.open();
    rig.wait_state(a, RenderState::Done);
    rig.none_started();
    assert_eq!(rig.state(b), RenderState::Queued);
    release.send(Ok(caps())).unwrap();
    assert_eq!(rig.next_started(), b);
    rig.wait_state(b, RenderState::Done);
}

#[test]
fn a_panicking_job_fails_and_leaves_no_files() {
    let rig = rig();
    rig.permits.open();
    let broken = rig.add("panic.mp4");
    let after = rig.add("after.mp4");
    rig.wait_state(broken, RenderState::Failed);
    let info = rig.info(broken);
    let message = info.message.unwrap_or_default();
    assert!(
        message.starts_with("Internal error: the fake runner broke"),
        "{message}"
    );
    assert!(
        !part_path(&info.destination, broken).exists(),
        "the part file is gone"
    );
    assert!(!temp_path(broken).exists(), "the working folder is gone");
    rig.wait_state(after, RenderState::Done);
    assert!(matches!(
        rig.queue.poll(),
        Some(RenderEvent::Finished { result: Err(_), .. })
    ));
}

#[test]
fn the_window_is_woken_for_changes_and_at_most_every_100ms_for_progress() {
    let rig = rig();
    let before = rig.wakes.load(Ordering::SeqCst);
    let a = rig.add("a.mp4");
    assert_eq!(rig.next_started(), a);
    until("the start's wake", || {
        rig.wakes.load(Ordering::SeqCst) > before
    });
    let started = rig.wakes.load(Ordering::SeqCst);
    // Ten frames at once: progress wakes are throttled.
    rig.permits.open();
    rig.wait_state(a, RenderState::Done);
    until("the end's wake", || {
        rig.wakes.load(Ordering::SeqCst) > started
    });
    let ended = rig.wakes.load(Ordering::SeqCst);
    assert!(
        ended - started <= 4,
        "ten frames in a moment woke the window {} times",
        ended - started
    );
    // The end itself is never throttled away.
    let b = rig.add("b.mp4");
    rig.wait_state(b, RenderState::Done);
    until("b's wakes", || {
        rig.wakes.load(Ordering::SeqCst) >= ended + 2
    });
}

#[test]
fn finished_events_are_bounded_and_the_newest_kept() {
    let rig = rig();
    rig.permits.open();
    let mut ids = Vec::new();
    for index in 0..40 {
        ids.push(rig.add(&format!("fail-{index}.mp4")));
        if index % 10 == 9 {
            let last = *ids.last().unwrap();
            rig.wait_state(last, RenderState::Failed);
            rig.queue.clear_finished();
        }
    }
    let told: Vec<Uuid> = std::iter::from_fn(|| rig.queue.poll())
        .map(|RenderEvent::Finished { id, .. }| id)
        .collect();
    assert_eq!(told.len(), MAX_EVENTS);
    assert_eq!(told, ids[ids.len() - MAX_EVENTS..]);
}

#[test]
fn shutdown_cancels_everything_and_refuses_more() {
    let rig = rig();
    let a = rig.add("a.mp4");
    let b = rig.add("b.mp4");
    assert_eq!(rig.next_started(), a);
    // The running job's part file, as the engine would have it.
    let part = part_path(&rig.dir.join("a.mp4"), a);
    fs::write(&part, b"partial").unwrap();
    rig.queue.shutdown();
    assert_eq!(rig.state(a), RenderState::Cancelled);
    assert_eq!(rig.state(b), RenderState::Cancelled);
    assert!(!part.exists(), "the part file is removed");
    assert_eq!(rig.killed.load(Ordering::SeqCst), 1);
    assert!(!rig.queue.busy());
    assert!(matches!(
        rig.queue.enqueue(rig.job("c.mp4")),
        Err(EnqueueError::Stopped(_))
    ));
    assert!(!rig.queue.retry(b));
    rig.none_started();
}

#[test]
fn headless_shutdown_waits_for_the_runners_resources_to_drop() {
    struct Resource(Arc<AtomicBool>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let resource = Resource(dropped.clone());
    let queue = RenderQueue::with_parts(
        || {},
        Arc::new(|| Ok(caps())),
        Box::new(move |_, _| {
            let _held = &resource;
            Finish::Cancelled
        }),
    );
    queue.shutdown_and_join();
    assert!(dropped.load(Ordering::Acquire));
    queue.shutdown_and_join();
}
