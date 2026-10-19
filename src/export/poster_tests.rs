//! The export sheet's poster: which output frame it is of, at what shape,
//! when it is asked for, which drawn poster is shown, and the picture the
//! `neptune-video-poster` thread draws through the export's renderer.
//!
//! Pictures come from memory. The drawing test reads back from a real
//! device and prints a line beginning `SKIP:` and passes without one.
use super::*;
use crate::compose::gpu_tests::device;
use editor_core::{
    AssetId, Clip, FrameRate, MediaKind, TrackKind,
    export::{ExportRange, Fit, Resolution, VideoSettings},
};
use std::{collections::HashMap, sync::Mutex as StdMutex};

const RED: [u8; 4] = [220, 40, 40, 255];

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

/// A 1920 × 1080, 30 fps project with one ten-second video clip that
/// decodes to `RED`, and the asset.
fn film() -> (Project, AssetId) {
    let mut project = Project::new("Poster");
    project.settings.width = 1920;
    project.settings.height = 1080;
    project.settings.frame_rate = FrameRate::default();
    let mut asset = Asset::new(
        PathBuf::from(format!("/nowhere/{}.mp4", Uuid::new_v4())),
        MediaKind::Video,
        seconds(10.0),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = 1000;
    let id = asset.id;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.duration = seconds(10.0);
    let track = (project.tracks.iter_mut())
        .find(|track| track.kind == TrackKind::Video)
        .expect("FIXTURE: a new project has a video track");
    Arc::make_mut(track).clips.push(clip);
    if let Err(error) = project.validate() {
        panic!("FIXTURE: the project is not valid: {error}");
    }
    (project, id)
}

fn tall() -> ExportSettings {
    ExportSettings {
        video: Some(VideoSettings {
            resolution: Resolution::Custom {
                width: 1080,
                height: 1920,
            },
            fit: Fit::Letterbox,
            ..VideoSettings::default()
        }),
        ..ExportSettings::default()
    }
}

fn key(project: &Project, settings: &ExportSettings, at: f64) -> PosterKey {
    poster_key(Uuid::nil(), 1, project, settings, seconds(at)).expect("a poster")
}

#[test]
fn the_poster_is_the_output_frame_at_the_playhead_at_a_small_size() {
    let (project, _) = film();
    let settings = tall();
    let poster = key(&project, &settings, 2.0);
    // The output's shape, its long side at the poster's, letterboxed as
    // the file is: the 16:9 canvas across a 9:16 picture.
    assert_eq!(poster.geometry.output, [360, 640]);
    assert_eq!(
        poster.geometry,
        Geometry::new([1920, 1080], [360, 640], Fit::Letterbox)
    );
    assert_eq!(poster.geometry.drawn, [360, 203]);
    assert_eq!(poster.at, seconds(2.0));
    // The frame the playhead is in, at the output's rate.
    let mid = key(&project, &settings, 2.02);
    assert_eq!(mid.at, FrameRate::default().frame_time(60));
    // A small output is not drawn larger.
    let mut small = ExportSettings::default();
    small.video.as_mut().unwrap().resolution = Resolution::Custom {
        width: 320,
        height: 180,
    };
    assert_eq!(key(&project, &small, 2.0).geometry.output, [320, 180]);

    // Outside the range, the range's first or last frame.
    let mut ranged = tall();
    ranged.range = ExportRange::Span {
        start: seconds(4.0),
        end: seconds(6.0),
    };
    assert_eq!(key(&project, &ranged, 1.0).at, seconds(4.0));
    let last = key(&project, &ranged, 9.0).at;
    assert!(last < seconds(6.0) && last >= seconds(5.9), "{last:?}");
    assert_eq!(key(&project, &ranged, 9.0).geometry.output, [360, 640]);

    // Sound only, or nothing to export, has no poster.
    let sound = ExportSettings {
        video: None,
        ..ExportSettings::default()
    };
    assert!(poster_key(Uuid::nil(), 1, &project, &sound, Time::ZERO).is_none());
    let empty = Project::new("Empty");
    assert!(poster_key(Uuid::nil(), 1, &empty, &settings, Time::ZERO).is_none());
    // What changes the picture changes the key; what does not, does not.
    let mut codec = settings.clone();
    codec.video.as_mut().unwrap().codec = editor_core::export::VideoCodec::Vp9;
    assert_eq!(key(&project, &codec, 2.0), poster);
    let mut fill = settings.clone();
    fill.video.as_mut().unwrap().fit = Fit::Fill;
    assert_ne!(key(&project, &fill, 2.0), poster);
    assert_ne!(
        poster_key(Uuid::nil(), 2, &project, &settings, seconds(2.0)),
        Some(poster),
        "another revision is another picture"
    );
}

/// No source: the tests that use it ask for nothing.
struct Nothing;

impl PosterSource for Nothing {
    fn picture(&mut self, _: ClipId, asset: &Asset, _: Time, _: [u32; 2]) -> Result<Frame, String> {
        Err(format!("no picture of {}", asset.name))
    }
    fn retain(&mut self, _: &dyn Fn(ClipId) -> bool) {}
    fn canceller(&self) -> Box<dyn Fn() + Send + Sync> {
        Box::new(|| {})
    }
}

fn service() -> PosterService {
    PosterService::with_source(None, None, Arc::new(|| Box::new(Nothing)))
}

#[test]
fn a_poster_is_asked_for_once_the_settings_have_settled() {
    let (project, _) = film();
    let mut service = service();
    let start = Instant::now();
    let first = key(&project, &tall(), 2.0);
    // The first is asked for at once.
    assert_eq!(service.settle(first, start), Due::Now);
    service.sent = Some(first);
    assert_eq!(service.settle(first, start), Due::Sent);
    // A change waits until it has stayed the same for SETTLE.
    let fill = {
        let mut settings = tall();
        settings.video.as_mut().unwrap().fit = Fit::Fill;
        key(&project, &settings, 2.0)
    };
    let later = start + Duration::from_millis(500);
    assert_eq!(service.settle(fill, later), Due::In(SETTLE));
    assert_eq!(
        service.settle(fill, later + Duration::from_millis(100)),
        Due::In(Duration::from_millis(50))
    );
    // Another change starts the wait again.
    let moved = key(&project, &tall(), 3.0);
    let again = later + Duration::from_millis(140);
    assert_eq!(service.settle(moved, again), Due::In(SETTLE));
    assert_eq!(service.settle(moved, again + SETTLE), Due::Now);
}

#[test]
fn only_the_poster_of_the_latest_request_is_shown() {
    let ctx = egui::Context::default();
    let (project, _) = film();
    let mut service = service();
    let old = key(&project, &tall(), 2.0);
    let new = key(&project, &tall(), 3.0);
    let picture = |key: PosterKey| Published {
        key,
        picture: Ok(Picture {
            size: key.geometry.output,
            rgba: vec![9; (key.geometry.output[0] * key.geometry.output[1] * 4) as usize],
        }),
    };
    service.sent = Some(new);
    // The older one came back after the newer one was sent: stale.
    service.receive(&ctx, picture(old));
    assert!(service.shown.is_none(), "a stale poster is thrown away");
    service.receive(&ctx, picture(new));
    let (shown, texture) = service.shown.as_ref().expect("the latest is shown");
    assert_eq!(*shown, new);
    assert_eq!(texture.size(), [360, 640]);
    // A failure says why and keeps nothing stale.
    let newer = key(&project, &tall(), 4.0);
    service.sent = Some(newer);
    service.receive(
        &ctx,
        Published {
            key: newer,
            picture: Err("no device".to_owned()),
        },
    );
    assert_eq!(service.problem(), Some("no device"));
    // The poster shown is offered for the same shape only.
    let shape = service.show_at(
        &ctx,
        Some(newer),
        &Arc::new(project.clone()),
        &tall(),
        Instant::now(),
    );
    assert!(
        shape.is_some(),
        "the last poster stays while the next is drawn"
    );
    let mut wide = ExportSettings::default();
    wide.video.as_mut().unwrap().resolution = Resolution::P1080;
    let other = key(&project, &wide, 4.0);
    service.unstarted = true;
    assert!(
        service
            .show_at(&ctx, Some(other), &Arc::new(project), &wide, Instant::now())
            .is_none(),
        "a poster of another shape is not stretched"
    );
    service.stop();
    assert!(service.shown.is_none() && !service.running());
}

/// What a source was asked for: the source time and the size.
type Asked = Arc<StdMutex<Vec<(Time, [u32; 2])>>>;

/// Pictures from memory, shared with the test that reads what was asked.
struct Memory {
    frames: HashMap<AssetId, Frame>,
    asked: Asked,
}

impl PosterSource for Memory {
    fn picture(
        &mut self,
        _: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String> {
        self.asked.lock().unwrap().push((time, size));
        (self.frames.get(&asset.id).cloned()).ok_or_else(|| format!("no picture of {}", asset.name))
    }
    fn retain(&mut self, _: &dyn Fn(ClipId) -> bool) {}
    fn canceller(&self) -> Box<dyn Fn() + Send + Sync> {
        Box::new(|| {})
    }
}

#[test]
fn the_thread_draws_the_output_frame_with_the_export_s_bars() {
    let Some(gpu) = device() else {
        return;
    };
    let (project, asset) = film();
    let project = Arc::new(project);
    let red = Frame {
        width: 64,
        height: 36,
        rgba: RED.repeat(64 * 36).into(),
    };
    let asked: Asked = Arc::new(StdMutex::new(Vec::new()));
    let frames = HashMap::from([(asset, red)]);
    let source: SourceMaker = Arc::new({
        let asked = Arc::clone(&asked);
        move || {
            Box::new(Memory {
                frames: frames.clone(),
                asked: Arc::clone(&asked),
            })
        }
    });
    let worker = PosterWorker::start(Some((gpu.0.clone(), gpu.1.clone())), None, source, || {})
        .expect("the thread starts");
    let settings = tall();
    let poster = key(&project, &settings, 2.0);
    worker.ask(PosterRequest {
        key: poster,
        project: Arc::clone(&project),
        settings: settings.clone(),
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let published = loop {
        if let Some(published) = worker.take() {
            break published;
        }
        assert!(Instant::now() < deadline, "the poster is drawn in time");
        thread::sleep(Duration::from_millis(5));
    };
    worker.join();
    assert_eq!(published.key, poster);
    let picture = published.picture.expect("the poster is drawn");
    assert_eq!(picture.size, [360, 640]);
    assert_eq!(picture.rgba.len(), 360 * 640 * 4);
    let at = |x: usize, y: usize| -> [u8; 4] {
        let from = (y * 360 + x) * 4;
        std::array::from_fn(|channel| picture.rgba[from + channel])
    };
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(&b).all(|(a, b)| a.abs_diff(*b) <= 2);
    // The canvas across the middle, black bars above and below it.
    assert!(near(at(180, 320), RED), "{:?}", at(180, 320));
    assert!(near(at(180, 10), [0, 0, 0, 255]), "{:?}", at(180, 10));
    assert!(near(at(180, 630), [0, 0, 0, 255]), "{:?}", at(180, 630));
    // The original was asked for at the playhead's frame, no larger than
    // the poster draws it.
    let asked = asked.lock().unwrap();
    assert!(!asked.is_empty());
    assert!(
        asked
            .iter()
            .all(|(time, size)| *time == seconds(2.0) && size[0] <= 360),
        "{asked:?}"
    );
}

#[test]
fn stopping_ends_the_thread() {
    let worker = PosterWorker::start(None, None, Arc::new(|| Box::new(Nothing)), || {})
        .expect("the thread starts");
    // Nothing asked: it waits, and ends when stopped.
    worker.join();
}
