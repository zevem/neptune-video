//! Real codec fixtures for bounded worker-owned cold-pipe preparation.
use super::*;
use crate::PreviewConsumer;
use std::{io::Write, process::Child, sync::Arc, time::Duration};

const SIZE: [u32; 2] = [64, 32];
const FRAMES: usize = 4;

struct Encoder(Child);

impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) struct Videos {
    pub(crate) pool: DecoderPool,
    pub(crate) processes: Processes,
    pub(crate) assets: Vec<Asset>,
    expected: Vec<Vec<Vec<u8>>>,
    _directory: tempfile::TempDir,
}

impl Videos {
    pub(crate) fn new(count: usize) -> Self {
        let directory = tempfile::tempdir().expect("owned warmup codec directory");
        let mut assets = Vec::new();
        let mut expected = Vec::new();
        for source in 0..count {
            let frames: Vec<_> = (0..FRAMES)
                .map(|frame| {
                    [source as u8 * 29, frame as u8 * 47, 113, 255]
                        .repeat((SIZE[0] * SIZE[1]) as usize)
                })
                .collect();
            let raw = directory.path().join(format!("source-{source}.rgba"));
            let mut file = File::create(&raw).expect("owned raw warmup source");
            for frame in &frames {
                file.write_all(frame).expect("complete warmup source frame");
            }
            drop(file);
            let path = directory.path().join(format!("source-{source}.nut"));
            let mut command = tool_command("ffmpeg");
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-f",
                    "rawvideo",
                    "-pixel_format",
                    "rgba",
                    "-video_size",
                    "64x32",
                    "-framerate",
                    "30",
                    "-i",
                ])
                .arg(&raw)
                .args([
                    "-an",
                    "-c:v",
                    "ffv1",
                    "-level",
                    "3",
                    "-pix_fmt",
                    "bgra",
                    "-threads",
                    "1",
                    "-filter_threads",
                    "1",
                    "-f",
                    "nut",
                ])
                .arg(&path);
            let mut encoder = Encoder(command.spawn().expect("owned warmup encoder"));
            assert!(
                encoder
                    .0
                    .wait()
                    .expect("warmup encoder completed")
                    .success()
            );
            drop(encoder);
            let rate = FrameRate::default();
            let mut asset = Asset::new(path, MediaKind::Video, rate.frame_time(FRAMES as i64));
            asset.width = SIZE[0];
            asset.height = SIZE[1];
            asset.frame_rate = Some(rate);
            asset.file_size = std::fs::metadata(&asset.path).unwrap().len();
            assets.push(asset);
            expected.push(frames);
        }
        let processes = Processes::default();
        Self {
            pool: DecoderPool::new(processes.clone()),
            processes,
            assets,
            expected,
            _directory: directory,
        }
    }

    pub(crate) fn job(&self, source: usize, frame: usize, consumer: PreviewConsumer) -> FrameJob {
        FrameJob {
            generation: 1,
            sequence: (source + frame * self.assets.len() + 1) as u64,
            asset: self.assets[source].clone(),
            time: FrameRate::default().frame_time(frame as i64),
            lane: Lane::Consumer(consumer),
            size: SIZE,
        }
    }

    pub(crate) fn assert_pixels(&self, frame: &Frame, source: usize, index: usize) {
        assert_eq!([frame.width, frame.height], SIZE);
        assert_eq!(frame.rgba.as_ref(), self.expected[source][index].as_slice());
    }

    pub(crate) fn image(&self) -> Asset {
        let path = self._directory.path().join("actual-still.png");
        image::RgbaImage::from_pixel(SIZE[0], SIZE[1], image::Rgba([13, 79, 191, 255]))
            .save(&path)
            .unwrap();
        let mut asset = Asset::new(path, MediaKind::Image, Time(1_000_000));
        asset.width = SIZE[0];
        asset.height = SIZE[1];
        asset.file_size = std::fs::metadata(&asset.path).unwrap().len();
        asset
    }
}

impl Drop for Videos {
    fn drop(&mut self) {
        self.pool.clear();
    }
}

fn child(pool: &DecoderPool, lane: Lane) -> Arc<std::sync::Mutex<Child>> {
    Arc::clone(
        &pool
            .decoders
            .iter()
            .find(|decoder| decoder.lane == lane)
            .expect("prepared lane retained")
            .process
            .child,
    )
}

fn exited(child: &Arc<std::sync::Mutex<Child>>) {
    assert!(child.lock().unwrap().try_wait().unwrap().is_some());
}

#[test]
fn five_admitted_pipes_start_before_any_requested_pixels_are_read() {
    let mut videos = Videos::new(5);
    let consumers: [PreviewConsumer; 5] = std::array::from_fn(|_| PreviewConsumer::new());
    videos
        .pool
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])]);
    for (source, consumer) in consumers.iter().copied().enumerate() {
        let job = videos.job(source, 0, consumer);
        assert!(videos.pool.prepare_video(&job, || true).unwrap());
    }
    assert_eq!(videos.pool.starts, 5);
    assert_eq!(videos.pool.decoders.len(), 5);
    assert!(videos.pool.cache.is_empty());
    assert!(
        videos
            .pool
            .decoders
            .iter()
            .all(|decoder| decoder.next_index == 0)
    );
    let children: Vec<_> = consumers
        .iter()
        .map(|consumer| child(&videos.pool, Lane::Consumer(*consumer)))
        .collect();
    for index in 0..FRAMES {
        for (source, consumer) in consumers.iter().copied().enumerate() {
            let job = videos.job(source, index, consumer);
            let frame = videos.pool.frame(&job, || true).unwrap();
            videos.assert_pixels(&frame, source, index);
            assert_eq!(videos.pool.starts, 5);
            assert!(Arc::ptr_eq(
                &children[source],
                &child(&videos.pool, job.lane)
            ));
        }
    }
    videos.pool.clear();
    for child in &children {
        exited(child);
    }
}

#[test]
fn preparation_leaves_compatible_pipes_and_cached_pixels_untouched() {
    let mut videos = Videos::new(1);
    let consumer = PreviewConsumer::new();
    let first_job = videos.job(0, 0, consumer);
    let first = videos.pool.frame(&first_job, || true).unwrap();
    let original = child(&videos.pool, first_job.lane);
    assert!(!videos.pool.prepare_video(&first_job, || true).unwrap());
    assert!(Arc::ptr_eq(&original, &child(&videos.pool, first_job.lane)));
    let next_job = videos.job(0, 1, consumer);
    assert!(!videos.pool.prepare_video(&next_job, || true).unwrap());
    assert!(Arc::ptr_eq(&original, &child(&videos.pool, next_job.lane)));
    assert_eq!(videos.pool.starts, 1);
    videos.pool.retain_lanes(&[]);
    exited(&original);
    assert!(!videos.pool.prepare_video(&first_job, || true).unwrap());
    assert!(videos.pool.decoders.is_empty());
    assert_eq!(videos.pool.starts, 1);
    let cached = videos.pool.frame(&first_job, || true).unwrap();
    assert!(Arc::ptr_eq(&first.rgba, &cached.rgba));
    assert!(videos.pool.prepare_video(&next_job, || true).unwrap());
    let next = videos.pool.frame(&next_job, || true).unwrap();
    videos.assert_pixels(&next, 0, 1);
    assert_eq!(videos.pool.starts, 2);
    assert!(
        videos.pool.has_read_ahead(&next_job),
        "preparing a small cold pipe retains read-ahead eligibility"
    );
}

#[test]
fn actual_still_images_do_not_start_a_video_pipe_during_preparation() {
    let mut videos = Videos::new(1);
    let mut job = videos.job(0, 0, PreviewConsumer::new());
    job.asset = videos.image();
    assert!(!videos.pool.prepare_video(&job, || true).unwrap());
    assert_eq!(videos.pool.starts, 0);
    let frame = videos.pool.frame(&job, || true).unwrap();
    assert_eq!(
        frame.rgba.as_ref(),
        [13, 79, 191, 255].repeat((SIZE[0] * SIZE[1]) as usize)
    );
    assert_eq!(videos.pool.starts, 0);
}

#[test]
fn preparation_never_evicts_foreign_pipes_from_a_full_protected_bank() {
    let mut videos = Videos::new(6);
    let consumers: [PreviewConsumer; 6] = std::array::from_fn(|_| PreviewConsumer::new());
    videos
        .pool
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])]);
    for (source, consumer) in consumers[..5].iter().copied().enumerate() {
        let job = videos.job(source, 0, consumer);
        assert!(videos.pool.prepare_video(&job, || true).unwrap());
    }
    let children: Vec<_> = consumers[..5]
        .iter()
        .map(|consumer| child(&videos.pool, Lane::Consumer(*consumer)))
        .collect();
    let overflow = videos.job(5, 0, consumers[5]);
    assert!(!videos.pool.prepare_video(&overflow, || true).unwrap());
    assert_eq!(videos.pool.starts, 5);
    assert_eq!(videos.pool.decoders.len(), 5);
    for (index, consumer) in consumers[..5].iter().copied().enumerate() {
        assert!(Arc::ptr_eq(
            &children[index],
            &child(&videos.pool, Lane::Consumer(consumer))
        ));
    }
}

#[test]
fn prepared_same_lane_seek_source_and_resize_replace_only_that_pipe() {
    let mut videos = Videos::new(6);
    let consumers: [PreviewConsumer; 5] = std::array::from_fn(|_| PreviewConsumer::new());
    for (source, consumer) in consumers.iter().copied().enumerate() {
        let job = videos.job(source, if source == 0 { 2 } else { 0 }, consumer);
        assert!(videos.pool.prepare_video(&job, || true).unwrap());
    }
    let children: Vec<_> = consumers
        .iter()
        .map(|consumer| child(&videos.pool, Lane::Consumer(*consumer)))
        .collect();
    let backwards = videos.job(0, 0, consumers[0]);
    assert!(videos.pool.prepare_video(&backwards, || true).unwrap());
    exited(&children[0]);
    assert_eq!(videos.pool.starts, 6);
    let backward_child = child(&videos.pool, backwards.lane);
    let mut changed = backwards.clone();
    changed.sequence += 100;
    changed.asset = videos.assets[5].clone();
    assert!(videos.pool.prepare_video(&changed, || true).unwrap());
    exited(&backward_child);
    assert_eq!(videos.pool.starts, 7);
    let replacement_child = child(&videos.pool, changed.lane);
    changed.size = [32, 16];
    changed.sequence += 1;
    assert!(videos.pool.prepare_video(&changed, || true).unwrap());
    exited(&replacement_child);
    assert_eq!(videos.pool.starts, 8);
    assert_eq!(videos.pool.decoders.len(), 5);
    let reference = scaled_reference(&changed);
    let resized = videos.pool.frame(&changed, || true).unwrap();
    assert_eq!([resized.width, resized.height], changed.size);
    assert_eq!(resized.rgba.as_ref(), reference.as_slice());
    assert_eq!(videos.pool.starts, 8);
    for (index, consumer) in consumers.iter().copied().enumerate().skip(1) {
        assert!(Arc::ptr_eq(
            &children[index],
            &child(&videos.pool, Lane::Consumer(consumer))
        ));
    }
}

fn scaled_reference(job: &FrameJob) -> Vec<u8> {
    let mut command = tool_command("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-threads",
            "2",
            "-i",
        ])
        .arg(&job.asset.path)
        .args(["-an", "-filter_threads", "1", "-vf"])
        .arg(format!(
            "scale={}:{}:flags=fast_bilinear,fps=30/1",
            job.size[0], job.size[1]
        ))
        .args([
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgba",
            "-threads:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
    crate::process::capture(
        "ffmpeg",
        &mut command,
        &Processes::default(),
        frame_bytes(job.size[0], job.size[1]).unwrap(),
    )
    .unwrap()
}

#[test]
fn obsolete_preparation_starts_nothing_or_reaps_a_spawn_superseded_before_adoption() {
    let mut videos = Videos::new(1);
    let job = videos.job(0, 0, PreviewConsumer::new());
    assert!(matches!(
        videos.pool.prepare_video(&job, || false),
        Err(MediaError::Cancelled)
    ));
    assert_eq!(videos.pool.starts, 0);
    let processes = videos.processes.clone();
    assert!(matches!(
        videos
            .pool
            .prepare_video(&job, || processes.started_tools() == 0),
        Err(MediaError::Cancelled)
    ));
    assert_eq!(videos.pool.starts, 1);
    assert!(videos.pool.decoders.is_empty());
    let deadline = Instant::now() + Duration::from_secs(2);
    while processes.retained_monitors() != 0 {
        assert!(
            Instant::now() < deadline,
            "cancelled prepared child monitor survived"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn retirement_and_generation_clear_reap_prepared_unread_pipes() {
    let mut videos = Videos::new(2);
    let consumers = [PreviewConsumer::new(), PreviewConsumer::new()];
    for (source, consumer) in consumers.iter().copied().enumerate() {
        let job = videos.job(source, 0, consumer);
        assert!(videos.pool.prepare_video(&job, || true).unwrap());
    }
    let keep = child(&videos.pool, Lane::Consumer(consumers[0]));
    let retire = child(&videos.pool, Lane::Consumer(consumers[1]));
    videos.pool.retain_lanes(&[Lane::Consumer(consumers[0])]);
    exited(&retire);
    assert_eq!(videos.pool.decoders.len(), 1);
    videos.pool.clear();
    exited(&keep);
    assert!(videos.pool.decoders.is_empty());
    assert!(videos.pool.cache.is_empty());
}
