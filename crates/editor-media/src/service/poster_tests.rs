//! Posters are decoded on their own worker: one that is slow, or never ends,
//! does not hold back a picture the viewer waits for.
use super::*;
use std::time::{Duration, Instant};

fn still(path: PathBuf) -> Asset {
    let mut asset = Asset::new(path, MediaKind::Image, Time(1_000_000));
    asset.width = 32;
    asset.height = 16;
    asset
}

/// A video whose file is a pipe: FFmpeg opens it, then waits for bytes.
#[cfg(unix)]
fn piped_video(path: PathBuf) -> Asset {
    let mut asset = Asset::new(path, MediaKind::Video, Time(1_000_000));
    asset.width = 64;
    asset.height = 32;
    asset.frame_rate = Some(editor_core::FrameRate {
        numerator: 30,
        denominator: 1,
    });
    asset
}

/// A live service's next event, waiting at most `seconds`.
fn next_event(service: &MediaService, seconds: u64) -> Option<MediaEvent> {
    let until = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(event) = service.poll() {
            return Some(event);
        }
        if Instant::now() >= until {
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// The library and timeline ask for posters as a project opens; the person
/// presses play at once. A 4K poster takes up to a second to decode, and
/// playback asks for a frame every few milliseconds. Here the poster's file
/// is a pipe that a writer holds open without writing, so its decode lasts
/// for as long as the test says: the viewer's picture must not wait for it.
#[cfg(unix)]
#[test]
fn a_poster_that_is_still_decoding_does_not_hold_back_the_viewer() {
    let temp = tempfile::tempdir().unwrap();
    let red = temp.path().join("red.png");
    image::RgbaImage::from_pixel(32, 16, image::Rgba([255, 0, 0, 255]))
        .save(&red)
        .unwrap();
    let pipe = temp.path().join("slow.nut");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    assert!(made.success());
    // Opening a pipe to write waits for its reader: that is FFmpeg decoding
    // the poster, which then waits for bytes that are not written.
    let (opened, writer) = std::sync::mpsc::channel();
    let path = pipe.clone();
    thread::spawn(move || {
        let held = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let _ = opened.send(held);
    });

    let service = MediaService::new(|| {}).unwrap();
    let poster = piped_video(pipe);
    service
        .request_poster(&poster, Time::ZERO, [16, 16])
        .unwrap();
    let held = writer
        .recv_timeout(Duration::from_secs(5))
        .expect("a worker began decoding the poster");

    let viewer = PreviewConsumer::new();
    service
        .request_preview(viewer, &still(red), Time::ZERO, [16, 16])
        .unwrap();
    let event = next_event(&service, 5);
    let Some(MediaEvent::FrameReady {
        consumer, frame, ..
    }) = event
    else {
        panic!("the viewer's picture waited behind a poster still decoding: {event:?}");
    };
    assert_eq!(consumer, Some(viewer));
    assert_eq!(&frame.rgba[..4], &[255, 0, 0, 255]);

    // The pipe ends: the poster fails on its own worker and says so.
    drop(held);
    let Some(MediaEvent::Error {
        asset_id, consumer, ..
    }) = next_event(&service, 5)
    else {
        panic!("the poster's failure was published");
    };
    assert_eq!((asset_id, consumer), (Some(poster.id), None));
}

#[test]
fn a_font_has_no_poster_and_is_refused_before_it_takes_a_turn() {
    let service = MediaService {
        shared: Arc::new(Shared::default()),
        processes: Processes::default(),
    };
    let font = Asset::new("Anton.ttf".into(), MediaKind::Font, Time(1));
    assert!(matches!(
        service.request_poster(&font, Time::ZERO, [320, 180]),
        Err(MediaError::Invalid(_))
    ));
    assert!(
        service
            .request_frame(&font, Time::ZERO, [320, 180])
            .is_err()
    );
    assert!(service.shared.queue.lock().unwrap().frames.is_empty());
}
