//! The camera's path measured from frames drawn here with a motion that is
//! known, the corrections made of it, the cache's bounds, the lane, and the
//! whole of it on a shaken file FFmpeg makes.
use super::*;
use crate::{
    analysis::{AnalysisService, SourceIdentity, SteadyEvent},
    import::asset_from_probe,
    process::tool_command,
};
use std::{
    process::Stdio,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const WIDTH: usize = 200;
const HEIGHT: usize = 160;

// A number in 0..1 for a cell of the ground, the same every time.
fn cell(x: i64, y: i64) -> f32 {
    let mut hash = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    hash = (hash ^ (hash >> 31)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash = (hash ^ (hash >> 29)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((hash ^ (hash >> 32)) & 0xffff) as f32 / 65_535.0
}

/// A ground with something to tell every place from every other: cells of
/// seven pixels, eased into one another, so it has a value between pixels.
fn ground(x: f32, y: f32) -> f32 {
    let (x, y) = (x / 7.0, y / 7.0);
    let (x0, y0) = (x.floor(), y.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (fx, fy) = (ease(x - x0), ease(y - y0));
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = cell(x0, y0) * (1.0 - fx) + cell(x0 + 1, y0) * fx;
    let bottom = cell(x0, y0 + 1) * (1.0 - fx) + cell(x0 + 1, y0 + 1) * fx;
    30.0 + 200.0 * (top * (1.0 - fy) + bottom * fy)
}

/// What a camera at `at` on the ground, turned by `turn` radians clockwise,
/// sees: the ground's place `p` is in the frame at `p - at`, turned about the
/// frame's centre the other way.
fn seen(at: [f32; 2], turn: f32) -> Gray {
    let (sin, cos) = turn.sin_cos();
    let centre = [WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5];
    let mut luma = Vec::with_capacity(WIDTH * HEIGHT);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (dx, dy) = (x as f32 + 0.5 - centre[0], y as f32 + 0.5 - centre[1]);
            // The frame's place turned by the camera's turn is the ground's.
            let px = centre[0] + cos * dx - sin * dy + at[0];
            let py = centre[1] + sin * dx + cos * dy + at[1];
            luma.push(ground(px, py).round().clamp(0.0, 255.0) as u8);
        }
    }
    Gray {
        width: WIDTH,
        height: HEIGHT,
        luma,
    }
}

// A number in -1..1 for a frame, the same every time: the shake.
fn shake(frame: usize, salt: i64) -> f32 {
    cell(frame as i64 * 131 + 17, salt) * 2.0 - 1.0
}

fn measured(frames: impl Iterator<Item = Gray>) -> Motion {
    let mut camera = Camera::new();
    for frame in frames {
        camera.push(&frame).unwrap();
    }
    camera.finish(FrameRate::default())
}

/// The path in pixels of the frame: across, down.
fn pixels(motion: &Motion, frame: usize) -> [f32; 2] {
    [
        motion.path[frame][0] * WIDTH as f32,
        motion.path[frame][1] * HEIGHT as f32,
    ]
}

#[test]
fn a_shaken_camera_is_measured_to_a_tenth_of_a_pixel() {
    // A camera that drifts and is knocked about between pixels, up to four
    // of them a frame.
    let at = |frame: usize| {
        let t = frame as f32;
        [
            300.0 + 0.8 * t + 3.7 * shake(frame, 1),
            200.0 - 0.3 * t + 2.9 * shake(frame, 2),
        ]
    };
    const FRAMES: usize = 24;
    let motion = measured((0..FRAMES).map(|frame| seen(at(frame), 0.0)));
    assert_eq!(motion.path.len(), FRAMES);
    assert_eq!(motion.unmeasured, 0);
    let mut worst = 0f32;
    for frame in 0..FRAMES {
        // The picture goes the other way than the camera.
        let truth = [at(0)[0] - at(frame)[0], at(0)[1] - at(frame)[1]];
        let found = pixels(&motion, frame);
        worst = worst
            .max((found[0] - truth[0]).abs())
            .max((found[1] - truth[1]).abs());
        assert!(
            motion.path[frame][2].abs() < 0.002,
            "{:?}",
            motion.path[frame]
        );
        assert!(
            motion.path[frame][3].abs() < 0.002,
            "{:?}",
            motion.path[frame]
        );
    }
    eprintln!(
        "MEASURE: camera path of {FRAMES} synthetic frames {WIDTH}x{HEIGHT}: worst error {worst:.3} px over the whole path"
    );
    // The bound on what is left of a known shake: the sum of every step's
    // error, a tenth of a pixel over the whole path.
    assert!(worst < 0.1, "path off by {worst} px");
}

#[test]
fn a_camera_that_turns_is_measured_with_its_turn() {
    // A degree and a half either way, with a shift.
    let turn = |frame: usize| 0.026 * (frame as f32 * 0.9).sin();
    let at = |frame: usize| [300.0 + 1.5 * shake(frame, 3), 200.0 + 1.5 * shake(frame, 4)];
    const FRAMES: usize = 14;
    let motion = measured((0..FRAMES).map(|frame| seen(at(frame), turn(frame))));
    assert_eq!(motion.unmeasured, 0);
    for frame in 0..FRAMES {
        // The picture turns the other way than the camera.
        let truth = turn(0) - turn(frame);
        let found = motion.path[frame][2];
        assert!(
            (found - truth).abs() < 0.002,
            "frame {frame}: turned {found}, the camera {truth}"
        );
        assert!(motion.path[frame][3].abs() < 0.004);
    }
}

#[test]
fn a_part_of_the_picture_that_moves_by_itself_is_left_out() {
    // A patch in a corner goes its own way; the rest goes the camera's.
    let frame = |index: usize| {
        let mut frame = seen([300.0 + 2.0 * index as f32, 200.0], 0.0);
        let own = 9 * index;
        for y in 40..90 {
            for x in 40..100 {
                frame.luma[y * WIDTH + x] =
                    ground(900.0 + x as f32, 700.0 + (y + own) as f32).round() as u8;
            }
        }
        frame
    };
    let motion = measured((0..4).map(frame));
    for index in 0..4 {
        let found = pixels(&motion, index);
        assert!((found[0] + 2.0 * index as f32).abs() < 0.15, "{found:?}");
        assert!(found[1].abs() < 0.15, "{found:?}");
    }
}

#[test]
fn an_even_picture_says_nothing_and_a_small_one_is_refused() {
    let flat = Gray {
        width: WIDTH,
        height: HEIGHT,
        luma: vec![90; WIDTH * HEIGHT],
    };
    let motion = measured([flat.clone(), flat.clone(), flat].into_iter());
    assert_eq!(motion.path, vec![[0.0; 4]; 3]);
    assert_eq!(motion.unmeasured, 2);

    let small = Gray {
        width: 24,
        height: 20,
        luma: vec![0; 480],
    };
    assert!(Camera::new().push(&small).is_err());
}

/// The root of the mean square of what a path has over itself smoothed, in
/// pixels of the frame: how much it shakes.
fn jitter(motion: &Motion, extent: [f32; 2]) -> f32 {
    let deviation = motion.deviation(0.5);
    let count = motion.path.len();
    // The ends are left out: there the smoothing has one side only.
    let inner = count / 6..count - count / 6;
    let sum: f32 = (inner.clone())
        .map(|frame| {
            let smooth = motion.smoothed(frame, deviation);
            let over = [
                (motion.path[frame][0] - smooth[0]) * extent[0],
                (motion.path[frame][1] - smooth[1]) * extent[1],
            ];
            over[0] * over[0] + over[1] * over[1]
        })
        .sum();
    (sum / inner.len() as f32).sqrt()
}

#[test]
fn what_is_left_of_a_known_shake_is_under_a_fifth_of_a_pixel() {
    // A camera that pans evenly and shakes about the pan.
    const FRAMES: usize = 48;
    let pan = |frame: usize| [300.0 + 1.1 * frame as f32, 200.0 + 0.4 * frame as f32];
    let knock = |frame: usize| [3.0 * shake(frame, 5), 2.5 * shake(frame, 6)];
    let at = |frame: usize| {
        let (pan, knock) = (pan(frame), knock(frame));
        [pan[0] + knock[0], pan[1] + knock[1]]
    };
    let motion = measured((0..FRAMES).map(|frame| seen(at(frame), 0.0)));
    let extent = [WIDTH as f32, HEIGHT as f32];
    // Where the picture truly is in each frame once it is corrected: where
    // the camera put it, moved by the correction.
    let truth = Motion {
        rate: motion.rate,
        path: (0..FRAMES)
            .map(|frame| {
                let fix = motion.fix(frame, 0.5);
                [
                    (at(0)[0] - at(frame)[0]) / extent[0] + fix.shift[0],
                    (at(0)[1] - at(frame)[1]) / extent[1] + fix.shift[1],
                    0.0,
                    0.0,
                ]
            })
            .collect(),
        unmeasured: 0,
    };
    let shaken = Motion {
        rate: motion.rate,
        path: (0..FRAMES)
            .map(|frame| {
                [
                    (at(0)[0] - at(frame)[0]) / extent[0],
                    (at(0)[1] - at(frame)[1]) / extent[1],
                    0.0,
                    0.0,
                ]
            })
            .collect(),
        unmeasured: 0,
    };
    let (before, after) = (jitter(&shaken, extent), jitter(&truth, extent));
    // What is left from frame to frame: the corrected picture's steps
    // against the even pan's.
    let step = |path: &[[f32; 4]], frame: usize| {
        let dx = (path[frame][0] - path[frame - 1][0]) * extent[0] + 1.1;
        let dy = (path[frame][1] - path[frame - 1][1]) * extent[1] + 0.4;
        dx.hypot(dy)
    };
    let inner = FRAMES / 6..FRAMES - FRAMES / 6;
    let worst = |path: &[[f32; 4]]| {
        (inner.clone())
            .map(|frame| step(path, frame))
            .fold(0f32, f32::max)
    };
    eprintln!(
        "MEASURE: synthetic shake of {FRAMES} frames {WIDTH}x{HEIGHT} at strength 0.5: jitter {before:.3} px RMS before, {after:.3} px RMS after; worst step off the pan {:.3} px before, {:.3} px after",
        worst(&shaken.path),
        worst(&truth.path)
    );
    assert!(before > 1.5, "the fixture shakes by {before} px");
    // The bound: what the corrected picture still has over its own smooth
    // path is under a fifth of a pixel, RMS.
    assert!(after < 0.2, "{after} px RMS left of {before}");
}

// ---- Corrections -------------------------------------------------------------

fn level(path: Vec<[f32; 4]>) -> Motion {
    Motion {
        rate: FrameRate::default(),
        path,
        unmeasured: 0,
    }
}

#[test]
fn a_correction_takes_a_frame_from_the_path_to_the_smooth_one() {
    // One frame knocked aside on a path that is otherwise still.
    let mut path = vec![[0.0; 4]; 61];
    path[30] = [0.05, -0.02, 0.01, 0.0];
    let motion = level(path);
    let fix = motion.fix(30, 0.5);
    // The window is 7.5 frames of deviation: the knock is all but averaged
    // away, and the correction all but undoes it.
    assert!((fix.shift[0] + 0.05).abs() < 0.004, "{fix:?}");
    assert!((fix.shift[1] - 0.02).abs() < 0.002, "{fix:?}");
    assert!((fix.turn + 0.01).abs() < 0.001, "{fix:?}");
    // Its neighbours are moved a little towards it, and far frames not.
    assert!(motion.fix(29, 0.5).shift[0] > 0.0);
    assert_eq!(motion.fix(0, 0.5), Fix::default());
    // An even pan is left as it is, up to both its ends.
    let pan = level((0..40).map(|f| [0.01 * f as f32, 0.0, 0.0, 0.0]).collect());
    for frame in [0, 1, 5, 20, 38, 39] {
        assert!(pan.fix(frame, 1.0).shift[0].abs() < 1e-5, "frame {frame}");
    }
    // No smoothness and no frame are no correction.
    assert_eq!(motion.fix(30, 0.0), Fix::default());
    assert_eq!(motion.fix(99, 0.5), Fix::default());
    // A longer window averages it further away.
    assert!(motion.fix(30, 1.0).shift[0] < fix.shift[0]);

    // Between two frames the correction is a line between theirs.
    let rate = motion.rate;
    let (a, b) = (motion.fix(29, 0.5), motion.fix(30, 0.5));
    let half = Time((rate.frame_time(29).0 + rate.frame_time(30).0) / 2);
    let between = motion.fix_at(half, 0.5);
    assert!((between.shift[0] - 0.5 * (a.shift[0] + b.shift[0])).abs() < 1e-4);
    assert_eq!(motion.fix_at(rate.frame_time(30), 0.5), b);
    // A time past the path's end is its last frame.
    assert_eq!(
        motion.fix_at(Time::from_seconds(60.0), 0.5),
        motion.fix(60, 0.5)
    );
}

#[test]
fn a_picture_is_read_where_its_correction_says() {
    // Moved right by a tenth of its width: what is shown at the middle is
    // read a tenth to the left.
    let moved = Fix {
        shift: [0.1, 0.0],
        ..Fix::default()
    };
    let read = moved.sampling(1.0, 16.0 / 9.0);
    assert_eq!(read.matrix, [1.0, 0.0, 0.0, 1.0]);
    assert!((read.at([0.5, 0.5])[0] - 0.4).abs() < 1e-6);
    // Enlarged twice about the centre: the left edge shows the quarter.
    let zoomed = Fix::default().sampling(2.0, 1.0);
    assert!((zoomed.at([0.0, 0.5])[0] - 0.25).abs() < 1e-6);
    assert_eq!(Fix::default().sampling(1.0, 2.0), Sampling::IDENTITY);
    // Turned a quarter clockwise, a square frame: what is shown on the
    // right of the centre was above it.
    let turned = Fix {
        turn: std::f32::consts::FRAC_PI_2,
        ..Fix::default()
    };
    let at = turned.sampling(1.0, 1.0).at([0.75, 0.5]);
    assert!(
        (at[0] - 0.5).abs() < 1e-5 && (at[1] - 0.25).abs() < 1e-5,
        "{at:?}"
    );
    // In a wide frame a turn keeps its shape: a quarter of the width
    // across is the same pixels as that many down.
    let at = turned.sampling(1.0, 2.0).at([0.75, 0.5]);
    assert!((at[0] - 0.5).abs() < 1e-5 && at[1].abs() < 1e-5, "{at:?}");
}

#[test]
fn a_correction_is_held_to_what_its_crop_hides() {
    let aspect = 16.0 / 9.0;
    let wide = Fix {
        shift: [0.08, -0.03],
        turn: 0.02,
        grow: 0.0,
    };
    // Uncropped, nothing can be corrected without showing a border.
    assert_eq!(wide.fitted(1.0, aspect), Fix::default());
    // A crop too small takes the part of it that fits, and reads inside.
    let part = wide.fitted(1.1, aspect);
    assert!(part.shift[0] > 0.01 && part.shift[0] < wide.shift[0]);
    assert!((part.shift[1] / part.shift[0] - wide.shift[1] / wide.shift[0]).abs() < 1e-4);
    assert!(part.sampling(1.1, aspect).inside());
    assert!(!wide.sampling(1.1, aspect).inside());
    // A crop that is large enough takes it whole.
    assert_eq!(wide.fitted(1.4, aspect), wide);
}

#[test]
fn the_crop_suggested_hides_every_frame_of_the_stretch_and_no_more() {
    let path: Vec<[f32; 4]> = (0..90)
        .map(|frame| {
            [
                0.03 * shake(frame, 7),
                0.02 * shake(frame, 8),
                0.01 * shake(frame, 9),
                0.0,
            ]
        })
        .collect();
    let motion = level(path);
    let aspect = 16.0 / 9.0;
    let (from, until) = (Time::ZERO, Time::from_seconds(3.0));
    let zoom = motion.crop_zoom(from, until, 0.5, aspect);
    assert!(zoom > 1.02 && zoom < 1.3, "{zoom}");
    assert_eq!(zoom, (zoom * 100.0).round() / 100.0);
    for frame in 0..90 {
        let fix = motion.fix(frame, 0.5);
        assert_eq!(fix.fitted(zoom, aspect), fix, "frame {frame}");
    }
    // A hundredth less shows a border somewhere.
    assert!((0..90).any(|frame| {
        let fix = motion.fix(frame, 0.5);
        !fix.sampling(zoom - 0.011, aspect).inside()
    }));
    // A still path needs none, and one that cannot be hidden takes the most.
    assert_eq!(
        level(vec![[0.0; 4]; 30]).crop_zoom(from, until, 0.5, aspect),
        1.0
    );
    let wild = level(
        (0..30)
            .map(|f| [0.6 * shake(f, 1), 0.0, 0.0, 0.0])
            .collect(),
    );
    assert_eq!(wild.crop_zoom(from, until, 1.0, aspect), 2.0);
    // A stretch of one frame is that frame's.
    let one = motion.crop_zoom(
        Time::from_seconds(1.0),
        Time::from_seconds(1.0),
        0.5,
        aspect,
    );
    assert!((1.0..=zoom).contains(&one));
}

// ---- The cache -----------------------------------------------------------------

fn video_at(path: &Path, bytes: usize) -> Asset {
    fs::write(path, vec![7u8; bytes]).unwrap();
    let mut asset = Asset::new(path.into(), MediaKind::Video, Time::from_seconds(2.0));
    asset.width = 320;
    asset.height = 240;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = bytes as u64;
    asset
}

fn some_motion(frames: usize) -> Motion {
    Motion {
        rate: FrameRate::default(),
        path: (0..frames)
            .map(|frame| [frame as f32 * 0.001, -0.002, 0.01, 0.0])
            .collect(),
        unmeasured: 3,
    }
}

#[test]
fn a_path_kept_is_read_back_until_its_video_changes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SteadyCache::at(dir.path().join("steady"));
    let asset = video_at(&dir.path().join("shot.mkv"), 100);
    // Nothing is kept, and asking makes no directory.
    assert_eq!(cache.load(&asset), Steadied::Missing);
    assert!(!cache.dir.exists());

    let motion = some_motion(60);
    cache.store(&asset, &Stamp::of(&asset), &motion).unwrap();
    assert_eq!(
        cache.load(&asset),
        Steadied::Ready(Arc::new(motion.clone()))
    );
    assert_eq!(cache.held().1, 1);
    // Another asset of the same file reads the same path: it is the file's.
    let mut twin = asset.clone();
    twin.id = Uuid::new_v4();
    assert!(matches!(cache.load(&twin), Steadied::Ready(_)));

    // The project's own facts of the video are part of what it is.
    let mut longer = asset.clone();
    longer.duration = Time::from_seconds(3.0);
    assert_eq!(cache.load(&longer), Steadied::Stale);
    // The file changed in place: another size, or only another time.
    fs::write(&asset.path, vec![7u8; 101]).unwrap();
    assert_eq!(cache.load(&asset), Steadied::Stale);
    fs::write(&asset.path, vec![7u8; 100]).unwrap();
    let later = SystemTime::now() + Duration::from_secs(90);
    (fs::File::options().write(true).open(&asset.path).unwrap())
        .set_modified(later)
        .unwrap();
    assert_eq!(cache.load(&asset), Steadied::Stale);
    // And gone.
    fs::remove_file(&asset.path).unwrap();
    assert_eq!(cache.load(&asset), Steadied::Stale);

    // Measured again, it is the new file's.
    let asset = video_at(&dir.path().join("shot.mkv"), 100);
    cache.store(&asset, &Stamp::of(&asset), &motion).unwrap();
    assert!(matches!(cache.load(&asset), Steadied::Ready(_)));
    assert_eq!(cache.held().1, 1);
}

#[test]
fn a_damaged_file_or_one_of_another_measurement_is_no_path() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SteadyCache::at(dir.path().join("steady"));
    let asset = video_at(&dir.path().join("shot.mkv"), 100);
    let stamp = Stamp::of(&asset);
    cache.store(&asset, &stamp, &some_motion(20)).unwrap();
    let file = cache.file(&asset);
    let whole = fs::read(&file).unwrap();

    // Cut short, one byte turned, grown, and empty.
    for damaged in [
        whole[..whole.len() - 5].to_vec(),
        {
            let mut bytes = whole.clone();
            bytes[whole.len() / 2] ^= 0x10;
            bytes
        },
        [whole.clone(), vec![0; 16]].concat(),
        Vec::new(),
    ] {
        fs::write(&file, damaged).unwrap();
        assert_eq!(cache.load(&asset), Steadied::Missing);
    }
    // Whole, but of another version of the measurement.
    let mut other = whole.clone();
    other[8..12].copy_from_slice(&(STEADY_ALGORITHM + 1).to_le_bytes());
    let body = other.len() - 8;
    let sum = fnv(&other[..body]);
    other[body..].copy_from_slice(&sum.to_le_bytes());
    fs::write(&file, other).unwrap();
    assert_eq!(cache.load(&asset), Steadied::Missing);
    // Whole, with a number that is none.
    let mut path = some_motion(20);
    path.path[4][1] = f32::NAN;
    fs::write(&file, SteadyCache::encode(&asset, &stamp, &path)).unwrap();
    assert_eq!(cache.load(&asset), Steadied::Missing);
    fs::write(&file, whole).unwrap();
    assert!(matches!(cache.load(&asset), Steadied::Ready(_)));
}

#[test]
fn the_cache_is_held_to_its_bounds_and_gives_up_what_was_read_longest_ago() {
    let dir = tempfile::tempdir().unwrap();
    let assets: Vec<Asset> = (0..6)
        .map(|index| video_at(&dir.path().join(format!("shot{index}.mkv")), 50 + index))
        .collect();
    let motion = some_motion(100);
    let one = SteadyCache::encode(&assets[0], &Stamp::of(&assets[0]), &motion).len() as u64;
    // Room for three files by their count.
    let cache = SteadyCache {
        dir: dir.path().join("steady"),
        bytes: u64::MAX,
        entries: 3,
    };
    let aged = |asset: &Asset, seconds: u64| {
        let at = SystemTime::now() - Duration::from_secs(1000 - seconds);
        (fs::File::options()
            .write(true)
            .open(cache.file(asset))
            .unwrap())
        .set_modified(at)
        .unwrap();
    };
    for (index, asset) in assets[..3].iter().enumerate() {
        cache.store(asset, &Stamp::of(asset), &motion).unwrap();
        aged(asset, index as u64);
    }
    assert_eq!(cache.held(), (3 * one, 3));
    // The oldest is read, and so is the newest of them now.
    assert!(matches!(cache.load(&assets[0]), Steadied::Ready(_)));
    let read = fs::metadata(cache.file(&assets[0]))
        .unwrap()
        .modified()
        .unwrap();
    assert!(read.elapsed().unwrap() < Duration::from_secs(100));
    aged(&assets[0], 500);
    cache
        .store(&assets[3], &Stamp::of(&assets[3]), &motion)
        .unwrap();
    aged(&assets[3], 600);
    assert_eq!(cache.held().1, 3);
    assert!(cache.file(&assets[0]).exists());
    assert!(!cache.file(&assets[1]).exists(), "the one read longest ago");
    assert!(cache.file(&assets[2]).exists());
    assert!(cache.file(&assets[3]).exists());

    // Room for two files by their bytes: the two read longest ago go.
    let cache = SteadyCache {
        bytes: 2 * one + one / 2,
        entries: 100,
        ..cache
    };
    cache
        .store(&assets[4], &Stamp::of(&assets[4]), &motion)
        .unwrap();
    let (bytes, files) = cache.held();
    assert!(
        bytes <= cache.bytes && files == 2,
        "{bytes} bytes in {files}"
    );
    assert!(
        cache.file(&assets[4]).exists(),
        "what was just written stays"
    );
    assert!(cache.file(&assets[3]).exists());
    assert!(!cache.file(&assets[2]).exists());
    // A file larger than all the room is still kept: it is all there is.
    let cache = SteadyCache { bytes: 10, ..cache };
    cache
        .store(&assets[5], &Stamp::of(&assets[5]), &motion)
        .unwrap();
    assert_eq!(cache.held(), (one, 1));
    assert!(matches!(cache.load(&assets[5]), Steadied::Ready(_)));
    // Nothing but the cache's own files is touched.
    fs::write(cache.dir.join("notes.txt"), b"mine").unwrap();
    cache
        .store(&assets[0], &Stamp::of(&assets[0]), &motion)
        .unwrap();
    assert!(cache.dir.join("notes.txt").exists());
    assert_eq!(cache.held().1, 1);
}

// ---- The lane ----------------------------------------------------------------

/// A lane whose measuring waits for the gate, or for being given up.
struct Gated {
    service: AnalysisService,
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Arc<AtomicUsize>,
    wakes: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
    asset: Asset,
    cache: SteadyCache,
}

impl Gated {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let asset = video_at(&dir.path().join("shot.mkv"), 64);
        let cache = SteadyCache::at(dir.path().join("steady"));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let started = Arc::new(AtomicUsize::new(0));
        let wakes = Arc::new(AtomicUsize::new(0));
        let (held, begun, counted) = (Arc::clone(&gate), Arc::clone(&started), Arc::clone(&wakes));
        let service = AnalysisService::with_runners(
            Arc::new(move || {
                counted.fetch_add(1, Ordering::AcqRel);
            }),
            Processes::default(),
            Arc::new(|_, _| Err(MediaError::Cancelled)),
            Arc::new(move |job: &SteadyJob, control: &Control| {
                begun.fetch_add(1, Ordering::AcqRel);
                control.progress(0.25);
                control.progress(0.5);
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
                let motion = Arc::new(some_motion(job.asset.file_size as usize));
                let unkept = (job.cache)
                    .store(&job.asset, &Stamp::of(&job.asset), &motion)
                    .err();
                Ok((motion, unkept))
            }),
        )
        .unwrap();
        Self {
            service,
            gate,
            started,
            wakes,
            _dir: dir,
            asset,
            cache,
        }
    }

    fn job(&self, project: u128) -> SteadyJob {
        SteadyJob {
            project: Uuid::from_u128(project),
            asset: self.asset.clone(),
            cache: self.cache.clone(),
        }
    }

    fn open(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }

    fn wait(&self, what: &str, mut until: impl FnMut() -> bool) {
        let limit = Instant::now() + Duration::from_secs(10);
        while !until() {
            assert!(Instant::now() < limit, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn events(&self) -> Vec<SteadyEvent> {
        self.wait("the lane to be idle", || !self.service.busy());
        std::iter::from_fn(|| self.service.poll_steady()).collect()
    }
}

#[test]
fn a_measuring_says_how_far_it_is_and_answers_with_what_it_read() {
    let lane = Gated::new();
    let request = lane.service.steady(lane.job(7)).unwrap();
    lane.wait("the job to start", || {
        lane.started.load(Ordering::Acquire) == 1
    });
    lane.wait("its progress", || lane.wakes.load(Ordering::Acquire) >= 2);
    // One figure waits, the newest.
    match lane.service.poll_steady() {
        Some(SteadyEvent::Progress { request: of, done }) => {
            assert_eq!((of, done), (request, 0.5));
        }
        other => panic!("{other:?}"),
    }
    assert!(lane.service.poll_steady().is_none());
    // The other queue hears nothing of it.
    assert!(lane.service.poll().is_none());
    lane.open();
    let events = lane.events();
    let [
        SteadyEvent::Done {
            request: of,
            project,
            asset_id,
            source,
            measured: true,
            result: Ok(Steadied::Ready(motion)),
            unkept: None,
        },
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert_eq!((*of, *project), (request, Uuid::from_u128(7)));
    assert_eq!(*asset_id, lane.asset.id);
    assert_eq!(*source, SourceIdentity::of(&lane.asset));
    assert_eq!(motion.path.len(), 64);
    // And it was kept.
    assert!(matches!(lane.cache.load(&lane.asset), Steadied::Ready(_)));
}

#[test]
fn a_measuring_given_up_or_replaced_answers_nothing() {
    let lane = Gated::new();
    let first = lane.service.steady(lane.job(1)).unwrap();
    lane.wait("the job to start", || {
        lane.started.load(Ordering::Acquire) == 1
    });
    lane.service.cancel(first);
    assert!(lane.events().is_empty());
    assert!(matches!(lane.cache.load(&lane.asset), Steadied::Missing));

    // A second for the same video takes the first's place under way.
    let first = lane.service.steady(lane.job(1)).unwrap();
    lane.wait("the job to start", || {
        lane.started.load(Ordering::Acquire) == 2
    });
    let second = lane.service.steady(lane.job(2)).unwrap();
    assert_ne!(first, second);
    lane.open();
    let done: Vec<(u64, Uuid)> = (lane.events().into_iter())
        .filter_map(|event| match event {
            SteadyEvent::Done {
                request, project, ..
            } => Some((request, project)),
            SteadyEvent::Progress { .. } => None,
        })
        .collect();
    assert_eq!(done, vec![(second, Uuid::from_u128(2))]);

    // Another project: everything is given up at once.
    *lane.gate.0.lock().unwrap() = false;
    lane.service.steady(lane.job(3)).unwrap();
    lane.service.load_steady(lane.job(3)).unwrap();
    lane.service.cancel_all();
    assert!(lane.events().is_empty());
}

#[test]
fn a_kept_path_is_read_without_waiting_for_a_video_being_measured() {
    let lane = Gated::new();
    let other = video_at(&lane._dir.path().join("other.mkv"), 32);
    let kept = some_motion(12);
    (lane.cache)
        .store(&other, &Stamp::of(&other), &kept)
        .unwrap();

    let measuring = lane.service.steady(lane.job(5)).unwrap();
    lane.wait("the job to start", || {
        lane.started.load(Ordering::Acquire) == 1
    });
    let reading = (lane.service)
        .load_steady(SteadyJob {
            asset: other.clone(),
            ..lane.job(5)
        })
        .unwrap();
    let missing = lane.service.load_steady(lane.job(5)).unwrap();
    // Both are answered while the measuring still waits at its gate.
    let mut answers = Vec::new();
    lane.wait("the readings", || {
        while let Some(event) = lane.service.poll_steady() {
            if let SteadyEvent::Done {
                request,
                measured,
                result,
                source,
                ..
            } = event
            {
                answers.push((request, measured, result.unwrap(), source));
            }
        }
        answers.len() == 2
    });
    assert_eq!(lane.started.load(Ordering::Acquire), 1);
    assert_eq!(
        answers,
        vec![
            (
                reading,
                false,
                Steadied::Ready(Arc::new(kept)),
                SourceIdentity::of(&other)
            ),
            (
                missing,
                false,
                Steadied::Missing,
                SourceIdentity::of(&lane.asset)
            ),
        ]
    );
    lane.service.cancel(measuring);

    // The readings that wait are bounded, the newest of a video in place
    // of the older, and only a video is asked about.
    let mut still = lane.job(5);
    still.asset.kind = MediaKind::Image;
    assert!(matches!(
        lane.service.steady(still.clone()),
        Err(MediaError::Invalid(_))
    ));
    assert!(matches!(
        lane.service.load_steady(still),
        Err(MediaError::Invalid(_))
    ));
}

// ---- FFmpeg ----------------------------------------------------------------------

fn ffmpeg_available() -> bool {
    tool_command("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Where the fixture's camera is in a frame, in whole pixels of the ground,
/// as FFmpeg's crop filter is told and cuts it.
fn shaken_by(frame: usize) -> [f32; 2] {
    let n = frame as f64;
    [
        (40.0 + 0.5 * n + 7.0 * (n * 1.3).sin() + 4.0 * (n * 2.9).sin()).floor() as f32,
        (40.0 + 6.0 * (n * 1.7).cos() + 3.0 * (n * 3.1).sin()).floor() as f32,
    ]
}

/// A 320 by 240 window on the ground, two seconds at 30 frames a second,
/// shaken by up to eleven pixels across and nine down about a slow pan.
fn shaky(dir: &Path) -> Asset {
    let ground_file = dir.join("ground.png");
    let picture = image::GrayImage::from_fn(480, 360, |x, y| {
        image::Luma([ground(x as f32, y as f32).round() as u8])
    });
    picture.save(&ground_file).unwrap();
    let path = dir.join("shaky.mkv");
    let status = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-loop", "1"])
        .args(["-framerate", "30", "-i"])
        .arg(&ground_file)
        .args(["-vf"])
        .arg(
            "format=gray,crop=320:240:\
             x='floor(40+0.5*n+7*sin(n*1.3)+4*sin(n*2.9))':\
             y='floor(40+6*cos(n*1.7)+3*sin(n*3.1))':exact=1",
        )
        .args(["-frames:v", "60", "-c:v", "ffv1"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let probe = r#"{"format":{"duration":"2"},"streams":[{"codec_type":"video","width":320,"height":240,"avg_frame_rate":"30/1"}]}"#;
    let mut asset = asset_from_probe(&path, probe.as_bytes()).unwrap();
    asset.file_size = fs::metadata(&path).unwrap().len();
    asset
}

fn await_done(service: &AnalysisService, request: u64) -> (bool, Result<Steadied, String>, f32) {
    let limit = Instant::now() + Duration::from_secs(120);
    let mut said = 0.0;
    loop {
        match service.poll_steady() {
            Some(SteadyEvent::Progress { request: of, done }) if of == request => {
                assert!(done >= said, "progress went back from {said} to {done}");
                said = done;
            }
            Some(SteadyEvent::Done {
                request: of,
                measured,
                result,
                unkept,
                ..
            }) if of == request => {
                assert_eq!(unkept, None);
                return (measured, result, said);
            }
            Some(_) => {}
            None => {
                assert!(Instant::now() < limit, "timed out waiting for the path");
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[test]
fn a_shaken_file_is_measured_kept_and_steadied() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the shaken stabilization fixture was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let asset = shaky(dir.path());
    let cache = SteadyCache::at(dir.path().join("cache"));
    let service = AnalysisService::new(|| {}).unwrap();
    let job = SteadyJob {
        project: Uuid::from_u128(11),
        asset: asset.clone(),
        cache: cache.clone(),
    };

    // Nothing is kept yet.
    let asked = service.load_steady(job.clone()).unwrap();
    assert_eq!(await_done(&service, asked).1, Ok(Steadied::Missing));

    let began = Instant::now();
    let asked = service.steady(job.clone()).unwrap();
    let (measured, result, said) = await_done(&service, asked);
    let took = began.elapsed();
    let Ok(Steadied::Ready(motion)) = result else {
        panic!("{result:?}");
    };
    assert!(measured && said > 0.9);
    assert_eq!(motion.path.len(), 60);
    assert_eq!(motion.unmeasured, 0);
    assert_eq!(motion.rate, FrameRate::default());
    // The path is the fixture's, to a fifth of a pixel over two seconds.
    let extent = [320.0, 240.0];
    let mut worst = 0f32;
    for frame in 0..60 {
        let (first, now) = (shaken_by(0), shaken_by(frame));
        for axis in 0..2 {
            let found = motion.path[frame][axis] * extent[axis];
            worst = worst.max((found - (first[axis] - now[axis])).abs());
        }
    }
    assert!(worst < 0.2, "the path is off by {worst} px");

    // Kept, and read back the same by another asking.
    let asked = service.load_steady(job.clone()).unwrap();
    let (measured, result, _) = await_done(&service, asked);
    assert!(!measured);
    assert_eq!(result, Ok(Steadied::Ready(Arc::clone(&motion))));
    let (bytes, files) = cache.held();
    assert_eq!(files, 1);

    // Each frame read as its correction says, and the camera measured
    // again in what that shows: how much it still shakes.
    let aspect = 320.0 / 240.0;
    let smoothness = 0.5;
    let zoom = motion.crop_zoom(Time::ZERO, asset.duration, smoothness, aspect);
    assert!(zoom > 1.02 && zoom < 1.25, "crop {zoom}");
    let processes = Processes::default();
    let never = AtomicBool::new(false);
    let control = Control {
        cancel: &never,
        report: &|_| {},
    };
    let mut luma = Luma::open(&asset, 0, 60, &processes).unwrap();
    let mut steadied = Camera::new();
    let mut clamped = 0;
    for frame in 0..60 {
        assert!(luma.next(&control).unwrap());
        let fix = motion.fix(frame, smoothness);
        clamped += usize::from(fix.fitted(zoom, aspect) != fix);
        let read = fix.fitted(zoom, aspect).sampling(zoom, aspect);
        let from = &luma.gray;
        let (width, height) = (from.width as f32, from.height as f32);
        let shown = Gray {
            width: from.width,
            height: from.height,
            luma: (0..from.height)
                .flat_map(|y| {
                    (0..from.width).map(move |x| {
                        let at = read.at([(x as f32 + 0.5) / width, (y as f32 + 0.5) / height]);
                        (from.sample(at[0] * width - 0.5, at[1] * height - 0.5)).round() as u8
                    })
                })
                .collect(),
        };
        steadied.push(&shown).unwrap();
    }
    assert_eq!(clamped, 0, "the suggested crop hides every correction");
    let after = steadied.finish(motion.rate);
    assert_eq!(after.unmeasured, 0);
    // The picture is enlarged by the crop, so its pixels are too.
    let (before, left) = (
        jitter(&motion, extent),
        jitter(&after, extent.map(|side| side / zoom)),
    );
    eprintln!(
        "MEASURE: FFmpeg shaken fixture 320x240, 60 frames at 30 fps, strength {smoothness}, crop {zoom:.2}: jitter {before:.2} px RMS before, {left:.2} px RMS after ({:.0}x less); path off the fixture by {worst:.3} px at worst; measured in {:.0} ms ({:.1} ms a frame, FFmpeg included, unoptimized build unless said otherwise); cache {bytes} bytes",
        before / left.max(1e-3),
        took.as_secs_f64() * 1000.0,
        took.as_secs_f64() * 1000.0 / 60.0,
    );
    assert!(before > 3.0, "the fixture shakes by {before} px");
    assert!(left < before / 8.0, "{left} px RMS left of {before}");

    // The file changes: what is kept is of what it was.
    let mut bytes = fs::read(&asset.path).unwrap();
    bytes.push(0);
    fs::write(&asset.path, bytes).unwrap();
    let asked = service.load_steady(job).unwrap();
    assert_eq!(await_done(&service, asked).1, Ok(Steadied::Stale));
}

#[test]
fn a_measuring_of_a_file_stops_when_it_is_given_up() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; cancelling a stabilization process was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Twenty seconds of a test pattern: the job is given up at its first
    // word, a hundredth of the way in.
    let path = dir.path().join("long.mkv");
    let status = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=s=320x240:r=30")
        .args(["-t", "20", "-c:v", "mpeg4", "-q:v", "6"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let probe = r#"{"format":{"duration":"20"},"streams":[{"codec_type":"video","width":320,"height":240,"avg_frame_rate":"30/1"}]}"#;
    let mut asset = asset_from_probe(&path, probe.as_bytes()).unwrap();
    let cache = SteadyCache::at(dir.path().join("cache"));
    let service = AnalysisService::new(|| {}).unwrap();
    let asked = service
        .steady(SteadyJob {
            project: Uuid::from_u128(1),
            asset: asset.clone(),
            cache: cache.clone(),
        })
        .unwrap();
    let limit = Instant::now() + Duration::from_secs(60);
    loop {
        match service.poll_steady() {
            Some(SteadyEvent::Progress { done, .. }) => {
                assert!(
                    done < 0.5,
                    "the job was to be given up early, not at {done}"
                );
                break;
            }
            Some(other) => panic!("{other:?}"),
            None => {
                assert!(Instant::now() < limit, "the job said nothing");
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    service.cancel(asked);
    let limit = Instant::now() + Duration::from_secs(20);
    while service.busy() {
        assert!(Instant::now() < limit, "the job did not stop");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.poll_steady().is_none());
    assert_eq!(cache.held().1, 0);

    // A file that is not there is an answer.
    asset.path = dir.path().join("gone.mkv");
    let asked = service
        .steady(SteadyJob {
            project: Uuid::from_u128(1),
            asset,
            cache,
        })
        .unwrap();
    let (_, result, _) = await_done(&service, asked);
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn blocks_close_together_say_where_the_frame_went_and_not_how_it_turned() {
    // Six blocks that went as a frame turned a hundredth of a radian would.
    let turned = |places: &[[f32; 2]]| -> Vec<Went> {
        (places.iter())
            .map(|&[x, y]| Went {
                at: [x, y],
                by: [2.0 - 0.01 * y, 1.0 + 0.01 * x],
            })
            .collect()
    };
    // Over the frame, the turn is read.
    let over: Vec<[f32; 2]> = (0..6)
        .map(|index| {
            [
                (index % 3) as f32 * 120.0 - 120.0,
                (index / 3) as f32 * 100.0 - 50.0,
            ]
        })
        .collect();
    let step = agreed(&turned(&over), 216.0).expect("six blocks agree");
    assert!(
        (step.b - 0.01).abs() < 1e-4 && step.a.abs() < 1e-4,
        "{step:?}"
    );
    assert!((step.shift[0] - 2.0).abs() < 1e-3 && (step.shift[1] - 1.0).abs() < 1e-3);

    // In one corner of it, only where they went is: a turn read from so
    // short an arm would be made much of at the frame's centre.
    let corner: Vec<[f32; 2]> = (0..6)
        .map(|index| {
            [
                100.0 + (index % 3) as f32 * 32.0,
                40.0 + (index / 3) as f32 * 32.0,
            ]
        })
        .collect();
    let step = agreed(&turned(&corner), 216.0).expect("six blocks agree");
    assert_eq!((step.a, step.b), (0.0, 0.0));
    // The shift is that of the blocks, as nothing better is known.
    assert!((step.shift[0] - 1.44).abs() < 0.05, "{step:?}");
}

#[test]
fn two_things_moving_apart_are_not_read_as_a_frame_that_turns_or_grows() {
    // The upper rows of blocks went one way and the lower another, as a
    // picture with something large moving in it has them: a growth fits
    // them a little, and neither made one.
    let blocks: Vec<Went> = (0..15)
        .map(|index| {
            let (column, row) = (index % 5, index / 5);
            Went {
                at: [column as f32 * 70.0 - 140.0, row as f32 * 60.0 - 60.0],
                by: [4.0, if row < 2 { -3.0 } else { 3.0 }],
            }
        })
        .collect();
    let step = agreed(&blocks, 216.0).expect("most of the blocks agree");
    assert_eq!((step.a, step.b), (0.0, 0.0), "{step:?}");
    // Where most of them went.
    assert_eq!(step.shift, [4.0, -3.0]);
}
