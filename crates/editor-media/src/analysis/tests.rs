//! The tracking arithmetic on frames drawn here with motion that is known,
//! the lane's queue with a job that waits to be told, and the whole of it on
//! a file FFmpeg makes.
use super::*;
use crate::{import::asset_from_probe, process::tool_command};
use std::{
    process::Stdio,
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};

const WIDTH: usize = 96;
const HEIGHT: usize = 72;

/// A soft-edged patterned spot centred on `center`, on an even ground. It is
/// drawn from its own formula at every pixel, so a centre between pixels is
/// exactly where the picture is. `mix` turns its pattern into another.
fn scene(center: [f32; 2], mix: f32) -> Gray {
    let mut luma = Vec::with_capacity(WIDTH * HEIGHT);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (dx, dy) = (x as f32 - center[0], y as f32 - center[1]);
            let envelope = (-(dx * dx + dy * dy) / (2.0 * 6.0 * 6.0)).exp();
            let first = (0.55 * dx + 0.2 * dy).cos() * (0.3 * dy).cos();
            let second = (0.25 * dx - 0.6 * dy).sin() * (0.45 * dx).cos();
            let pattern = first * (1.0 - mix) + second * mix;
            let value = 120.0 + 110.0 * envelope * pattern;
            luma.push(value.round().clamp(0.0, 255.0) as u8);
        }
    }
    Gray {
        width: WIDTH,
        height: HEIGHT,
        luma,
    }
}

fn flat() -> Gray {
    Gray {
        width: WIDTH,
        height: HEIGHT,
        luma: vec![120; WIDTH * HEIGHT],
    }
}

fn region_at(center: [f32; 2], side: f32) -> Region {
    Region {
        center: [
            (center[0] + 0.5) / WIDTH as f32,
            (center[1] + 0.5) / HEIGHT as f32,
        ],
        size: [side / WIDTH as f32, side / HEIGHT as f32],
    }
}

/// Where the spot is in frame `index`: a steady drift between pixels with a
/// sway across it.
fn travel(index: usize) -> [f32; 2] {
    let t = index as f32;
    [24.3 + 1.37 * t, 30.6 + 0.21 * t + 3.0 * (t / 5.0).sin()]
}

fn pixels(center: [f32; 2]) -> [f32; 2] {
    [
        center[0] * WIDTH as f32 - 0.5,
        center[1] * HEIGHT as f32 - 0.5,
    ]
}

#[test]
fn a_region_is_followed_to_a_tenth_of_a_pixel() {
    // The stated bound: every frame's centre within a tenth of an analysis
    // pixel of where the spot was drawn, over 36 frames of motion between
    // pixels, with no error adding up along the way. (0.016 was measured.)
    const BOUND: f32 = 0.1;
    let mut path = Path::begin(&scene(travel(0), 0.0), region_at(travel(0), 28.0)).unwrap();
    for index in 1..36 {
        assert!(path.step(&scene(travel(index), 0.0)), "lost at {index}");
    }
    assert_eq!(path.centers.len(), 36);
    let mut worst = 0f32;
    for (index, center) in path.centers.iter().enumerate() {
        let (found, drawn) = (pixels(*center), travel(index));
        let error = (found[0] - drawn[0]).hypot(found[1] - drawn[1]);
        worst = worst.max(error);
    }
    eprintln!("worst tracking error: {worst:.3} px");
    assert!(worst <= BOUND, "worst error {worst} px");
    assert!(path.confidence > 0.95, "confidence {}", path.confidence);
}

#[test]
fn a_region_that_goes_is_lost_where_it_went_and_the_path_ends_there() {
    let mut path = Path::begin(&scene(travel(0), 0.0), region_at(travel(0), 28.0)).unwrap();
    for index in 1..10 {
        assert!(path.step(&scene(travel(index), 0.0)));
    }
    // The spot is gone: nothing in the frame is like it.
    assert!(!path.step(&flat()));
    // And it is not found again, were it to come back.
    assert!(!path.step(&scene(travel(11), 0.0)));
    let tracked = path.finish(12, |index| Time(index as i64 * 1000));
    assert_eq!(tracked.lost_at, Some(Time(10_000)));
    assert_eq!(tracked.keys.last().unwrap().source, Time(9000));
    let last = pixels(tracked.keys.last().unwrap().center);
    let drawn = travel(9);
    assert!((last[0] - drawn[0]).hypot(last[1] - drawn[1]) < 0.1);
    assert!(tracked.confidence >= LOST_BELOW);
}

#[test]
fn another_pattern_under_the_region_is_not_taken_for_it() {
    // A different spot where the first was: alike enough in outline to be
    // found, too unlike to be the region.
    let at = [40.0, 36.0];
    let mut path = Path::begin(&scene(at, 0.0), region_at(at, 28.0)).unwrap();
    let before = path.matcher.center;
    assert!(!path.step(&scene(at, 1.0)));
    assert_eq!(path.matcher.center, before);
}

#[test]
fn a_region_that_changes_slowly_is_followed_by_taking_its_patch_again() {
    let frames = 40;
    let mix = |index: usize| index as f32 / (frames - 1) as f32;
    let mut path = Path::begin(&scene(travel(0), 0.0), region_at(travel(0), 28.0)).unwrap();
    let first = path.matcher.template.clone();
    for index in 1..frames {
        let frame = scene(travel(index), mix(index));
        assert!(path.step(&frame), "lost at {index}");
    }
    // The patch it began with would not have found the last frame.
    assert_ne!(path.matcher.template, first);
    let last = scene(travel(frames - 1), 1.0);
    let half = path.matcher.half();
    let place = path.matcher.center;
    let mut original = Matcher {
        template: first,
        size: path.matcher.size,
        center: place,
        velocity: [0.0; 2],
        reach: path.matcher.reach,
    };
    let unlike = original
        .score(
            &last,
            (place[0] - half[0]).round() as i32,
            (place[1] - half[1]).round() as i32,
            1,
        )
        .unwrap();
    assert!(unlike < LOST_BELOW, "the first patch still scores {unlike}");
    assert!(original.step(&last) < LOST_BELOW);
    // Each taking of the patch may add a little: a pattern turning into
    // another has no one centre. A pixel and a half over the whole change.
    let found = pixels(*path.centers.last().unwrap());
    let drawn = travel(frames - 1);
    let error = (found[0] - drawn[0]).hypot(found[1] - drawn[1]);
    assert!(error < 1.5, "error {error} px");
    assert!((LOST_BELOW..REFRESH_BELOW + 0.2).contains(&path.confidence));
}

#[test]
fn an_even_region_is_refused_and_a_region_is_held_in_the_frame() {
    let error = Path::begin(&flat(), region_at([40.0, 30.0], 20.0))
        .err()
        .unwrap();
    assert!(error.contains("too even"), "{error}");
    // A region off the edge is moved inside, and a speck is made a box.
    let held = Region {
        center: [1.4, -0.2],
        size: [0.0001, f32::NAN],
    }
    .held();
    assert_eq!(held.size, [Region::MIN_SIDE; 2]);
    assert_eq!(
        held.center,
        [1.0 - Region::MIN_SIDE / 2.0, Region::MIN_SIDE / 2.0]
    );
    let path = Path::begin(&scene([4.0, 4.0], 0.0), region_at([0.0, 0.0], 20.0)).unwrap();
    let half = path.matcher.half();
    assert_eq!(path.matcher.center, half);
}

fn key(source: i64, x: f32, y: f32) -> TrackKey {
    TrackKey {
        source: Time(source),
        center: [x, y],
        scale: 1.0,
        rotation: 0.0,
    }
}

#[test]
fn a_path_is_thinned_to_what_a_line_does_not_say_and_to_its_limit() {
    // A straight path is its two ends.
    let straight: Vec<_> = (0..500)
        .map(|index| key(index * 1000, 0.1 + index as f32 * 0.001, 0.5))
        .collect();
    let thin = thinned(straight.clone(), 1e-5, MAX_TRACK_KEYS);
    assert_eq!(thin, vec![straight[0], straight[499]]);
    // A corner is kept.
    let mut bent = straight.clone();
    for (index, key) in bent.iter_mut().enumerate().skip(250) {
        key.center[1] = 0.5 + (index - 250) as f32 * 0.001;
    }
    let thin = thinned(bent.clone(), 1e-5, MAX_TRACK_KEYS);
    assert_eq!(thin, vec![bent[0], bent[250], bent[499]]);
    // A path that never runs straight keeps the limit, both ends, and its
    // order.
    let restless: Vec<_> = (0..5000)
        .map(|index| {
            key(
                index * 1000,
                0.5 + 0.3 * ((index * index) as f32).sin(),
                0.5,
            )
        })
        .collect();
    let thin = thinned(restless.clone(), 1e-5, MAX_TRACK_KEYS);
    assert_eq!(thin.len(), MAX_TRACK_KEYS);
    assert_eq!(thin[0], restless[0]);
    assert_eq!(*thin.last().unwrap(), restless[4999]);
    assert!(thin.windows(2).all(|pair| pair[0].source < pair[1].source));
    // Times that do not advance are not keys.
    let stuck = vec![key(0, 0.1, 0.1), key(0, 0.2, 0.2), key(5, 0.3, 0.1)];
    assert_eq!(thinned(stuck.clone(), 1e-5, 8), vec![stuck[0], stuck[2]]);
}

// ---- The lane -------------------------------------------------------------

fn video() -> Asset {
    let mut asset = Asset::new(
        "/media/shot.mkv".into(),
        MediaKind::Video,
        Time::from_seconds(4.0),
    );
    asset.width = 320;
    asset.height = 240;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = 1234;
    asset
}

fn job(target: Uuid, mark: i64) -> TrackJob {
    TrackJob {
        project: Uuid::from_u128(7),
        target,
        asset: video(),
        region: Region {
            center: [0.5, 0.5],
            size: [0.2, 0.2],
        },
        from: Time(mark),
        until: Time::from_seconds(2.0),
    }
}

/// A lane whose jobs wait for the gate, or for being given up, and answer
/// with the time they were asked from as their frame count.
struct Gated {
    service: AnalysisService,
    gate: Arc<(Mutex<bool>, Condvar)>,
    wakes: Arc<AtomicUsize>,
    started: Arc<AtomicUsize>,
}

impl Gated {
    fn new() -> Self {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let wakes = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let (held, counted, begun) = (Arc::clone(&gate), Arc::clone(&wakes), Arc::clone(&started));
        let service = AnalysisService::with_runner(
            Arc::new(move || {
                counted.fetch_add(1, Ordering::AcqRel);
            }),
            Processes::default(),
            Arc::new(move |job: &TrackJob, control: &Control| {
                begun.fetch_add(1, Ordering::AcqRel);
                for done in [0.1, 0.2, 0.3] {
                    control.progress(done);
                }
                let mut open = held.0.lock().unwrap();
                while !*open {
                    if control.cancelled() {
                        return Err(MediaError::Cancelled);
                    }
                    open = (held.1)
                        .wait_timeout(open, Duration::from_millis(2))
                        .unwrap()
                        .0;
                }
                Ok(Tracked {
                    keys: Vec::new(),
                    frames: job.from.0 as usize,
                    lost_at: None,
                    confidence: 1.0,
                })
            }),
        )
        .unwrap();
        Self {
            service,
            gate,
            wakes,
            started,
        }
    }

    fn open(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }

    fn wait(&self, what: &str, until: impl Fn() -> bool) {
        let limit = Instant::now() + Duration::from_secs(10);
        while !until() {
            assert!(Instant::now() < limit, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn started(&self) -> usize {
        self.started.load(Ordering::Acquire)
    }

    /// Every answer once the lane is idle, as `(request, frames)`, and the
    /// last progress read of each.
    fn answers(&self) -> Vec<(u64, usize)> {
        self.wait("the lane to be idle", || !self.service.busy());
        std::iter::from_fn(|| self.service.poll())
            .filter_map(|event| match event {
                AnalysisEvent::Tracked {
                    request, result, ..
                } => Some((request, result.unwrap().frames)),
                AnalysisEvent::Progress { .. } => None,
            })
            .collect()
    }
}

#[test]
fn an_answer_names_its_request_its_project_and_the_source_it_read() {
    let lane = Gated::new();
    let target = Uuid::from_u128(1);
    let request = lane.service.track(job(target, 5)).unwrap();
    lane.wait("the job to start", || lane.started() == 1);
    // Only the newest figure of a job waits.
    lane.wait("progress", || lane.wakes.load(Ordering::Acquire) >= 3);
    match lane.service.poll() {
        Some(AnalysisEvent::Progress { request: of, done }) => {
            assert_eq!(of, request);
            assert_eq!(done, 0.3);
        }
        other => panic!("{other:?}"),
    }
    assert!(lane.service.poll().is_none());
    let woken = lane.wakes.load(Ordering::Acquire);
    lane.open();
    lane.wait("the answer", || !lane.service.busy());
    assert!(lane.wakes.load(Ordering::Acquire) > woken);
    match lane.service.poll() {
        Some(AnalysisEvent::Tracked {
            request: of,
            project,
            target: for_target,
            asset_id: _,
            source,
            result,
        }) => {
            assert_eq!(of, request);
            assert_eq!(project, Uuid::from_u128(7));
            assert_eq!(for_target, target);
            assert_eq!(source, SourceIdentity::of(&video()));
            assert_eq!(result.unwrap().frames, 5);
        }
        other => panic!("{other:?}"),
    }
    // The source's identity is every fact a decoded frame depends on.
    let mut other = video();
    other.file_size += 1;
    assert_ne!(SourceIdentity::of(&other), SourceIdentity::of(&video()));
    // An idle lane wakes nobody.
    let woken = lane.wakes.load(Ordering::Acquire);
    thread::sleep(Duration::from_millis(30));
    assert_eq!(lane.wakes.load(Ordering::Acquire), woken);
}

#[test]
fn a_job_given_up_answers_nothing_waiting_or_under_way() {
    let lane = Gated::new();
    let running = lane.service.track(job(Uuid::from_u128(1), 1)).unwrap();
    lane.wait("the job to start", || lane.started() == 1);
    let waiting = lane.service.track(job(Uuid::from_u128(2), 2)).unwrap();
    let kept = lane.service.track(job(Uuid::from_u128(3), 3)).unwrap();
    lane.service.cancel(waiting);
    lane.service.cancel(running);
    // What the job under way had said is dropped with it.
    lane.wait("the next job to start", || lane.started() == 2);
    lane.open();
    assert_eq!(lane.answers(), vec![(kept, 3)]);
    assert_eq!(
        lane.started(),
        2,
        "a job given up while it waited never ran"
    );

    // Giving up everything, as for another project, leaves nothing to read.
    let lane = Gated::new();
    lane.service.track(job(Uuid::from_u128(1), 1)).unwrap();
    lane.wait("the job to start", || lane.started() == 1);
    lane.service.track(job(Uuid::from_u128(2), 2)).unwrap();
    lane.service.cancel_all();
    assert!(lane.service.poll().is_none());
    lane.wait("the lane to be idle", || !lane.service.busy());
    assert!(lane.service.poll().is_none());
    assert_eq!(lane.started(), 1);
}

#[test]
fn the_newest_job_for_a_target_takes_the_place_of_the_older() {
    let lane = Gated::new();
    let (one, two) = (Uuid::from_u128(1), Uuid::from_u128(2));
    lane.service.track(job(one, 1)).unwrap();
    lane.wait("the job to start", || lane.started() == 1);
    lane.service.track(job(two, 2)).unwrap();
    let newest_two = lane.service.track(job(two, 3)).unwrap();
    // The one under way is stopped by its successor too.
    let newest_one = lane.service.track(job(one, 4)).unwrap();
    lane.wait("the next job to start", || lane.started() == 2);
    lane.open();
    assert_eq!(lane.answers(), vec![(newest_two, 3), (newest_one, 4)]);
    assert_eq!(lane.started(), 3);
}

#[test]
fn the_queue_is_bounded_and_a_job_that_cannot_be_is_refused() {
    let lane = Gated::new();
    lane.service.track(job(Uuid::from_u128(100), 1)).unwrap();
    lane.wait("the job to start", || lane.started() == 1);
    for index in 0..ANALYSIS_QUEUE {
        lane.service
            .track(job(Uuid::from_u128(index as u128), 1))
            .unwrap();
    }
    assert!(matches!(
        lane.service.track(job(Uuid::from_u128(50), 1)),
        Err(MediaError::Busy)
    ));
    // One that takes the place of a waiting job is not one more.
    lane.service.track(job(Uuid::from_u128(0), 9)).unwrap();
    lane.open();
    assert_eq!(lane.answers().len(), ANALYSIS_QUEUE + 1);

    let refused = |change: &dyn Fn(&mut TrackJob)| {
        let mut asked = job(Uuid::from_u128(1), 1);
        change(&mut asked);
        matches!(lane.service.track(asked), Err(MediaError::Invalid(_)))
    };
    assert!(refused(&|job| job.asset.kind = MediaKind::Image));
    assert!(refused(&|job| job.asset.width = 0));
    assert!(refused(&|job| job.region.size = [0.0, 0.2]));
    assert!(refused(&|job| job.region.center = [f32::NAN, 0.2]));
    assert!(refused(&|job| job.until = job.from));
    assert!(!lane.service.busy());
}

// ---- FFmpeg ---------------------------------------------------------------

fn ffmpeg_available() -> bool {
    tool_command("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A white box of 40 pixels on grey, 320 by 240 at 30 frames a second, whose
/// corner is asked to be at `(40.5 + 60t, 60.5 + 20t)`. FFmpeg's overlay puts
/// it on whole pixels, and on even ones for this 4:2:0 picture: see
/// [`box_center`].
fn moving_box(path: &std::path::Path, seconds: u32) -> Asset {
    let status = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=c=0x404040:s=320x240:r=30")
        .args(["-f", "lavfi", "-i", "color=c=white:s=40x40:r=30"])
        .args([
            "-filter_complex",
            "[0][1]overlay=x='40.5+60*t':y='60.5+20*t'",
        ])
        .args(["-t", &seconds.to_string(), "-c:v", "ffv1"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
    let probe = format!(
        r#"{{"format":{{"duration":"{seconds}"}},"streams":[{{"codec_type":"video","width":320,"height":240,"avg_frame_rate":"30/1"}}]}}"#
    );
    let mut asset = asset_from_probe(path, probe.as_bytes()).unwrap();
    asset.file_size = std::fs::metadata(path).unwrap().len();
    asset
}

/// The centre of the box in a frame, in pixels of the picture: its corner
/// cut to a whole pixel and then to an even one.
fn box_center(frame: i64) -> [f32; 2] {
    let t = frame as f64 / 30.0;
    let corner = [40.5 + 60.0 * t, 60.5 + 20.0 * t].map(|at| (at as i64 & !1) as f32);
    [corner[0] + 20.0, corner[1] + 20.0]
}

fn box_job(asset: &Asset, until: f64) -> TrackJob {
    TrackJob {
        project: Uuid::from_u128(9),
        target: Uuid::from_u128(1),
        asset: asset.clone(),
        // The box with twelve pixels of ground around it.
        region: Region {
            center: [60.0 / 320.0, 80.0 / 240.0],
            size: [64.0 / 320.0, 64.0 / 240.0],
        },
        from: Time::ZERO,
        until: Time::from_seconds(until),
    }
}

/// The path at a time, as the project reads it: a line between keys.
fn along(keys: &[TrackKey], at: Time) -> [f32; 2] {
    let next = keys.partition_point(|key| key.source <= at);
    let Some(to) = keys.get(next) else {
        return keys[keys.len() - 1].center;
    };
    let from = keys[next - 1];
    let t = (at.0 - from.source.0) as f32 / (to.source.0 - from.source.0) as f32;
    [0, 1].map(|axis| from.center[axis] + (to.center[axis] - from.center[axis]) * t)
}

#[test]
fn a_box_moving_through_a_file_is_followed_from_its_frames() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the moving-box tracking fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let asset = moving_box(&temp.path().join("box.mkv"), 2);
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&wakes);
    let service = AnalysisService::new(move || {
        counted.fetch_add(1, Ordering::AcqRel);
    })
    .unwrap();
    let request = service.track(box_job(&asset, 1.5)).unwrap();
    let limit = Instant::now() + Duration::from_secs(120);
    let mut progress = Vec::new();
    let tracked = loop {
        assert!(Instant::now() < limit, "tracking did not end");
        match service.poll() {
            Some(AnalysisEvent::Progress { request: of, done }) => {
                assert_eq!(of, request);
                progress.push(done);
            }
            Some(AnalysisEvent::Tracked {
                request: of,
                project,
                asset_id,
                source,
                result,
                ..
            }) => {
                assert_eq!(of, request);
                assert_eq!(project, Uuid::from_u128(9));
                assert_eq!(asset_id, asset.id);
                assert_eq!(source, SourceIdentity::of(&asset));
                break result.unwrap();
            }
            None => thread::sleep(Duration::from_millis(2)),
        }
    };
    assert!(wakes.load(Ordering::Acquire) >= 1);
    assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(progress.iter().all(|done| (0.0..=1.0).contains(done)));
    // 1.5 seconds at 30 frames a second, to the end and never lost.
    assert_eq!(tracked.frames, 45);
    assert_eq!(tracked.lost_at, None);
    assert!(
        tracked.confidence > 0.9,
        "confidence {}",
        tracked.confidence
    );
    assert!((2..=45).contains(&tracked.keys.len()));
    assert_eq!(tracked.keys[0].source, Time::ZERO);
    assert_eq!(
        tracked.keys.last().unwrap().source,
        FrameRate::default().frame_time(44)
    );
    // Every frame within a quarter of a pixel of where the box was put.
    let mut worst = 0f32;
    for frame in 0..45 {
        let at = FrameRate::default().frame_time(frame);
        let drawn = box_center(frame);
        let found = along(&tracked.keys, at);
        let error = (found[0] * 320.0 - drawn[0]).hypot(found[1] * 240.0 - drawn[1]);
        worst = worst.max(error);
    }
    eprintln!("worst moving-box error: {worst:.3} px of 320");
    assert!(worst <= 0.25, "worst error {worst} px");
    assert!(
        tracked
            .keys
            .iter()
            .all(|key| key.scale == 1.0 && key.rotation == 0.0)
    );
}

#[test]
fn a_job_reading_a_file_stops_when_it_is_given_up() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; cancelling a tracking process was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let asset = moving_box(&temp.path().join("long.mkv"), 20);
    let service = AnalysisService::new(|| {}).unwrap();
    // The box leaves the frame after some five seconds: long enough.
    let request = service.track(box_job(&asset, 20.0)).unwrap();
    let limit = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < limit, "tracking did not start");
        match service.poll() {
            Some(AnalysisEvent::Progress { .. }) => break,
            Some(other) => panic!("{other:?}"),
            None => thread::sleep(Duration::from_millis(2)),
        }
    }
    let given_up = Instant::now();
    service.cancel(request);
    while service.busy() {
        assert!(
            given_up.elapsed() < Duration::from_secs(5),
            "the job did not stop"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.poll().is_none());
    // The lane takes the next job as usual.
    let next = service.track(box_job(&asset, 0.2)).unwrap();
    let limit = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < limit, "the next job did not end");
        match service.poll() {
            Some(AnalysisEvent::Tracked {
                request, result, ..
            }) => {
                assert_eq!(request, next);
                assert_eq!(result.unwrap().frames, 6);
                break;
            }
            _ => thread::sleep(Duration::from_millis(2)),
        }
    }
}

#[test]
fn a_file_that_is_not_there_is_an_answer_and_not_a_silence() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; a missing file's answer was not run");
        return;
    }
    let service = AnalysisService::new(|| {}).unwrap();
    let mut asked = job(Uuid::from_u128(1), 0);
    asked.asset.path = "/nonexistent/aster-analysis.mkv".into();
    service.track(asked).unwrap();
    let limit = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < limit);
        match service.poll() {
            Some(AnalysisEvent::Tracked { result, .. }) => {
                assert!(result.is_err());
                break;
            }
            _ => thread::sleep(Duration::from_millis(2)),
        }
    }
}
