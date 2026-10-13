use super::*;
use crate::PreviewConsumer;
use std::{cell::Cell, fs, io::Write, process::Child, time::Duration};

const SIZE: [u32; 2] = [16, 8];
const FRAMES: usize = 14;

struct OwnedEncoder(Child);

impl Drop for OwnedEncoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Video {
    pool: DecoderPool,
    asset: Asset,
    directory: tempfile::TempDir,
}

fn color(source: u8, frame: usize) -> [u8; 4] {
    [source, frame as u8 * 11, 73, 255]
}

fn pixels(source: u8, frame: usize, size: [u32; 2]) -> Vec<u8> {
    color(source, frame).repeat((size[0] * size[1]) as usize)
}

impl Video {
    fn new(frames: usize) -> Self {
        let directory = tempfile::tempdir().expect("owned prefetch fixture directory");
        let asset = Self::encode(directory.path(), "original", 17, frames);
        Self {
            pool: DecoderPool::new(Processes::default()),
            asset,
            directory,
        }
    }

    fn encode(directory: &std::path::Path, name: &str, source: u8, frames: usize) -> Asset {
        let input = directory.join(format!("{name}.rgba"));
        let mut raw = File::create(&input).expect("owned raw source");
        for index in 0..frames {
            raw.write_all(&pixels(source, index, SIZE))
                .expect("complete raw source frame");
        }
        drop(raw);
        let path = directory.join(format!("{name}.nut"));
        // Lossless solid frames make full pixels, scaling and frame order
        // independent of inter-frame prediction and interpolation rounding.
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
                "16x8",
                "-framerate",
                "30",
                "-i",
            ])
            .arg(input)
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
        let mut encoder = OwnedEncoder(command.spawn().expect("configured FFmpeg fixture encoder"));
        assert!(
            encoder
                .0
                .wait()
                .expect("owned encoder completion")
                .success()
        );
        let rate = FrameRate::default();
        let mut asset = Asset::new(path, MediaKind::Video, rate.frame_time(frames as i64));
        asset.width = SIZE[0];
        asset.height = SIZE[1];
        asset.frame_rate = Some(rate);
        asset.file_size = fs::metadata(&asset.path)
            .expect("encoded source metadata")
            .len();
        asset
    }

    fn job(&self, frame: usize) -> FrameJob {
        FrameJob {
            generation: 1,
            sequence: frame as u64 + 1,
            lane: Lane::Consumer(PreviewConsumer::new()),
            asset: self.asset.clone(),
            time: FrameRate::default().frame_time(frame as i64),
            size: SIZE,
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.pool.clear();
    }
}

fn future_deadline() -> Instant {
    // Tests verify decode semantics without relying on a loaded host completing
    // a tiny FFmpeg fixture inside the runtime's separate four-ms budget.
    Instant::now() + Duration::from_secs(2)
}

fn scaled_reference(asset: &Asset, frame: usize, size: [u32; 2]) -> Vec<u8> {
    // Independently index continuous output from source zero. Tiny fast-bilinear
    // previews can change even solid RGB values; do not assume identity scaling.
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
        .arg(&asset.path)
        .args(["-an", "-filter_threads", "1", "-vf"])
        .arg(format!(
            "scale={}:{}:flags=fast_bilinear,fps=30/1",
            size[0], size[1]
        ))
        .arg("-frames:v")
        .arg((frame + 1).to_string())
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    let bytes = frame_bytes(size[0], size[1]).unwrap();
    let raw = crate::process::capture(
        "ffmpeg",
        &mut command,
        &Processes::default(),
        bytes * (frame + 1),
    )
    .unwrap();
    assert_eq!(raw.len(), bytes * (frame + 1));
    raw[bytes * frame..].to_vec()
}

#[test]
fn prefetched_pixels_remain_exact_after_the_owned_pipe_is_evicted() {
    let mut video = Video::new(FRAMES);
    let job = video.job(0);
    let requested = video.pool.frame(&job, || true).unwrap();
    assert_eq!(requested.rgba.as_ref(), pixels(17, 0, SIZE));
    assert_eq!(
        video.pool.cache.len(),
        1,
        "requested decode performs no future reads"
    );
    assert!(video.pool.has_read_ahead(&job));
    video.pool.read_ahead(&job, future_deadline(), || true);
    assert!(!video.pool.has_read_ahead(&job), "hint is consumed once");
    assert_eq!(video.pool.cache.len(), 13, "twelve complete future frames");
    assert_eq!(video.pool.decoders[0].next_index, 13);
    video.pool.retain_lanes(&[]);
    assert!(video.pool.decoders.is_empty());
    let starts = video.pool.starts;
    for index in 1..=12 {
        let next = FrameJob {
            sequence: index as u64 + 1,
            time: FrameRate::default().frame_time(index),
            ..job.clone()
        };
        let cached = video.pool.frame(&next, || true).unwrap();
        assert_eq!(cached.rgba.as_ref(), pixels(17, index as usize, SIZE));
        assert!(!video.pool.has_read_ahead(&next));
        assert_eq!(
            video.pool.starts, starts,
            "cache hit survives pipe eviction"
        );
    }
    assert!(video.pool.bytes <= FRAME_CACHE_BYTES);
}

#[test]
fn cold_hint_is_exact_and_skips_protected_cached_and_existing_pipes() {
    let mut video = Video::new(FRAMES);
    let job = video.job(0);
    let Lane::Consumer(consumer) = job.lane else {
        unreachable!()
    };
    video
        .pool
        .set_preview_retention([Some(consumer), None, None]);
    video.pool.frame(&job, || true).unwrap();
    assert!(!video.pool.has_read_ahead(&job));
    video.pool.set_preview_retention([None; 3]);
    let next = FrameJob {
        sequence: 2,
        time: FrameRate::default().frame_time(1),
        ..job.clone()
    };
    video.pool.frame(&next, || true).unwrap();
    assert!(
        !video.pool.has_read_ahead(&next),
        "existing continuous pipe"
    );
    video.pool.frame(&job, || true).unwrap();
    assert!(
        !video.pool.has_read_ahead(&job),
        "cache hit is not a cold decode"
    );

    let changed_asset = Video::encode(video.directory.path(), "replacement", 201, FRAMES);
    let mut replaced = job.clone();
    replaced.asset.path = changed_asset.path;
    replaced.asset.file_size = changed_asset.file_size;
    let frame = video.pool.frame(&replaced, || true).unwrap();
    assert_eq!(frame.rgba.as_ref(), pixels(201, 0, SIZE));
    assert!(video.pool.has_read_ahead(&replaced));
    for changed in 0..7 {
        let mut other = replaced.clone();
        match changed {
            0 => other.generation += 1,
            1 => other.sequence += 1,
            2 => other.time = FrameRate::default().frame_time(1),
            3 => other.size = [8, 4],
            4 => other.asset.file_size += 1,
            5 => other.asset.id = AssetId::new_v4(),
            _ => other.lane = Lane::Consumer(PreviewConsumer::new()),
        }
        assert!(
            !video.pool.has_read_ahead(&other),
            "different hint identity {changed}"
        );
    }
    video
        .pool
        .set_preview_retention([Some(consumer), None, None]);
    assert!(
        !video.pool.has_read_ahead(&replaced),
        "new protection cancels eligibility"
    );
    video.pool.set_preview_retention([None; 3]);
    video.pool.read_ahead(&replaced, future_deadline(), || true);
    let next = FrameJob {
        sequence: 2,
        time: FrameRate::default().frame_time(1),
        ..replaced.clone()
    };
    let frame = video.pool.frame(&next, || true).unwrap();
    assert_eq!(frame.rgba.as_ref(), pixels(201, 1, SIZE));
    let resized = FrameJob {
        size: [8, 4],
        ..next
    };
    let frame = video.pool.frame(&resized, || true).unwrap();
    assert_eq!([frame.width, frame.height], [8, 4]);
    assert_eq!(
        frame.rgba.as_ref(),
        scaled_reference(&resized.asset, 1, [8, 4])
    );
    assert!(video.pool.has_read_ahead(&resized), "new fitted-size pipe");
}

#[test]
fn cancellation_after_a_complete_future_read_preserves_progress_without_obsolete_cache_insertion() {
    let mut video = Video::new(FRAMES);
    let job = video.job(0);
    video.pool.frame(&job, || true).unwrap();
    let checks = Cell::new(0);
    video.pool.read_ahead(&job, future_deadline(), || {
        let check = checks.get();
        checks.set(check + 1);
        check < 2 // Current at entry and before read; cancelled before insertion.
    });
    assert_eq!(
        video.pool.cache.len(),
        1,
        "obsolete future pixels are not inserted"
    );
    assert_eq!(
        video.pool.decoders[0].next_index, 2,
        "healthy pipe keeps completed progress"
    );
    assert!(!video.pool.has_read_ahead(&job));
    let next = FrameJob {
        sequence: 3,
        time: FrameRate::default().frame_time(2),
        ..job
    };
    let frame = video.pool.frame(&next, || true).unwrap();
    assert_eq!(frame.rgba.as_ref(), pixels(17, 2, SIZE));
    assert_eq!(video.pool.starts, 1);
}

#[test]
fn expired_budget_and_future_eof_preserve_the_requested_frame() {
    let mut video = Video::new(1);
    // Controlled mismatch forces future EOF, not a source-duration early exit.
    video.asset.duration = FrameRate::default().frame_time(FRAMES as i64);
    let job = video.job(0);
    let requested = video.pool.frame(&job, || true).unwrap();
    video.pool.read_ahead(&job, future_deadline(), || true);
    assert!(
        video.pool.decoders.is_empty(),
        "EOF pipe is dropped and reaped"
    );
    assert_eq!(video.pool.cache.len(), 1);
    let cached = video.pool.frame(&job, || true).unwrap();
    assert!(std::sync::Arc::ptr_eq(&requested.rgba, &cached.rgba));
    assert_eq!(cached.rgba.as_ref(), pixels(17, 0, SIZE));

    video.pool.clear();
    video.pool.frame(&job, || true).unwrap();
    let next_index = video.pool.decoders[0].next_index;
    video.pool.read_ahead(&job, Instant::now(), || true);
    assert_eq!(
        video.pool.decoders[0].next_index, next_index,
        "no expired read begins"
    );
    assert!(
        !video.pool.has_read_ahead(&job),
        "even an expired hint is consumed"
    );
    assert_eq!(
        video.pool.frame(&job, || true).unwrap().rgba.as_ref(),
        pixels(17, 0, SIZE)
    );
}

#[test]
fn prefetch_insertions_preserve_both_cache_limits() {
    for fill_bytes in [false, true] {
        let mut video = Video::new(FRAMES);
        let job = video.job(0);
        let dimensions = if fill_bytes { [4096, 1024] } else { SIZE };
        let bytes = frame_bytes(dimensions[0], dimensions[1]).unwrap();
        let seed = Frame::new(dimensions[0], dimensions[1], vec![41; bytes]).unwrap();
        for index in 0..if fill_bytes { 4 } else { 256 } {
            video.pool.cache_frame(
                Key {
                    asset: job.asset.id,
                    dimensions,
                    frame: 1000 + index,
                    source: Source::from(&job.asset),
                },
                seed.clone(),
                None,
            );
        }
        video.pool.frame(&job, || true).unwrap();
        video.pool.read_ahead(&job, future_deadline(), || true);
        assert!(video.pool.cache.len() <= 256);
        assert!(video.pool.bytes <= FRAME_CACHE_BYTES);
        assert_eq!(
            video.pool.bytes,
            video
                .pool
                .cache
                .iter()
                .map(|entry| entry.frame.byte_len())
                .sum::<usize>()
        );
        for index in 0..=12 {
            let next = FrameJob {
                sequence: index as u64 + 1,
                time: FrameRate::default().frame_time(index),
                ..job.clone()
            };
            assert_eq!(
                video.pool.frame(&next, || true).unwrap().rgba.as_ref(),
                pixels(17, index as usize, SIZE)
            );
        }
    }
}

#[test]
fn image_and_gif_requests_never_offer_speculation() {
    let directory = tempfile::tempdir().unwrap();
    let png = directory.path().join("still.png");
    image::RgbaImage::from_pixel(16, 8, image::Rgba(color(17, 0)))
        .save(&png)
        .unwrap();
    let gif = directory.path().join("animation.gif");
    let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&gif).unwrap());
    encoder
        .encode_frame(image::Frame::from_parts(
            image::RgbaImage::from_pixel(16, 8, image::Rgba(color(17, 0))),
            0,
            0,
            image::Delay::from_numer_denom_ms(100, 1),
        ))
        .unwrap();
    drop(encoder);
    let mut pool = DecoderPool::new(Processes::default());
    for (path, kind) in [(png, MediaKind::Image), (gif, MediaKind::Gif)] {
        let mut asset = Asset::new(path, kind, Time(100_000));
        asset.width = SIZE[0];
        asset.height = SIZE[1];
        let job = FrameJob {
            generation: 1,
            sequence: 1,
            lane: Lane::Consumer(PreviewConsumer::new()),
            asset,
            time: Time::ZERO,
            size: SIZE,
        };
        pool.frame(&job, || true).unwrap();
        assert!(!pool.has_read_ahead(&job));
        pool.read_ahead(&job, future_deadline(), || true);
        assert!(pool.decoders.is_empty());
    }
}
