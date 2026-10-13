use super::*;
use crate::PreviewConsumer;
use std::sync::Arc;

#[test]
fn equal_portrait_pixels_are_shared_between_consumers_without_dimension_identity_offsets() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("portrait.png");
    image::RgbaImage::from_pixel(8, 16, image::Rgba([17, 40, 80, 255]))
        .save(&path)
        .unwrap();
    let mut asset = Asset::new(path, MediaKind::Image, Time(1_000_000));
    asset.width = 8;
    asset.height = 16;
    let mut job = FrameJob {
        generation: 1,
        sequence: 1,
        lane: Lane::Consumer(PreviewConsumer::new()),
        size: [16, 16],
        asset,
        time: Time::ZERO,
    };
    let mut pool = DecoderPool::new(Processes::default());
    let first = pool.frame(&job, || true).unwrap();
    job.lane = Lane::Consumer(PreviewConsumer::new());
    job.time = Time(500_000);
    let second = pool.frame(&job, || true).unwrap();
    assert_eq!([first.width, first.height], [8, 16]);
    assert_eq!([second.width, second.height], [8, 16]);
    assert!(Arc::ptr_eq(&first.rgba, &second.rgba));
    assert_eq!(pool.cache.len(), 1);
    assert_eq!(&*first.rgba, [17, 40, 80, 255].repeat(8 * 16));
}

#[test]
fn retiring_one_gif_lane_releases_its_state_and_preserves_another_lane_and_shared_pixels() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("consumers.gif");
    let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
    for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
        encoder
            .encode_frame(image::Frame::from_parts(
                image::RgbaImage::from_pixel(8, 4, image::Rgba(color)),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
    }
    drop(encoder);
    let mut asset = Asset::new(path, MediaKind::Gif, Time(200_000));
    asset.width = 8;
    asset.height = 4;
    let a = Lane::Consumer(PreviewConsumer::new());
    let b = Lane::Consumer(PreviewConsumer::new());
    let mut job = FrameJob {
        generation: 1,
        sequence: 1,
        lane: a,
        size: [8, 4],
        asset,
        time: Time::ZERO,
    };
    let mut pool = DecoderPool::new(Processes::default());
    let red = pool.frame(&job, || true).unwrap();
    job.lane = b;
    job.time = Time(100_000);
    let green = pool.frame(&job, || true).unwrap();
    assert_eq!(pool.gifs.len(), 2);
    let before = pool.gif_state_bytes;
    pool.retain_lanes(&[b]);
    assert_eq!(pool.gifs.len(), 1);
    assert_eq!(pool.gifs[0].lane, b);
    assert!(pool.gif_state_bytes < before);
    assert!(Arc::ptr_eq(
        &green.rgba,
        &pool.frame(&job, || true).unwrap().rgba
    ));
    job.lane = Lane::Consumer(PreviewConsumer::new());
    job.time = Time::ZERO;
    assert!(Arc::ptr_eq(
        &red.rgba,
        &pool.frame(&job, || true).unwrap().rgba
    ));
    pool.retain_lanes(&[]);
    assert!(pool.gifs.is_empty());
    assert_eq!(pool.gif_state_bytes, 0);
}

fn resizable_gif() -> (tempfile::TempDir, FrameJob) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("resize.gif");
    let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
    for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
        encoder
            .encode_frame(image::Frame::from_parts(
                image::RgbaImage::from_pixel(16, 8, image::Rgba(color)),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
    }
    drop(encoder);
    let mut asset = Asset::new(path, MediaKind::Gif, Time(200_000));
    asset.width = 16;
    asset.height = 8;
    let job = FrameJob {
        generation: 1,
        sequence: 1,
        lane: Lane::Consumer(PreviewConsumer::new()),
        size: [8, 4],
        asset,
        time: Time::ZERO,
    };
    (temp, job)
}

#[test]
fn resizing_a_consumer_within_the_same_gif_interval_returns_the_new_fitted_dimensions() {
    let (_temp, mut job) = resizable_gif();
    let mut pool = DecoderPool::new(Processes::default());
    let before = pool.frame(&job, || true).unwrap();
    assert_eq!([before.width, before.height], [8, 4]);
    job.size = [16, 8];
    job.time = Time(50_000);
    let after = pool.frame(&job, || true).unwrap();
    assert_eq!(
        [after.width, after.height],
        [16, 8],
        "consumer resize must not return the old held GIF dimensions"
    );
    assert_eq!(&*after.rgba, [255, 0, 0, 255].repeat(16 * 8));
}

#[test]
fn forward_gif_resize_accounts_for_its_new_retained_preview_reservation() {
    let (_temp, mut job) = resizable_gif();
    let mut pool = DecoderPool::new(Processes::default());
    pool.frame(&job, || true).unwrap();
    job.size = [16, 8];
    job.time = Time(100_000);
    let after = pool.frame(&job, || true).unwrap();
    assert_eq!([after.width, after.height], [16, 8]);
    assert_eq!(&*after.rgba, [0, 255, 0, 255].repeat(16 * 8));
    assert_eq!(
        pool.gif_state_bytes,
        2 * 16 * 8 * 4,
        "new GIF preview dimensions must be reserved with its source canvas"
    );
}
