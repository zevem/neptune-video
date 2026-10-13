//! Publication and admission remain independent of speculative future reads.
use super::*;
use crate::Frame;

fn fixture() -> (MediaService, Asset, PreviewConsumer) {
    let service = MediaService {
        shared: Arc::new(Shared::default()),
        processes: Processes::default(),
    };
    let mut asset = Asset::new(
        PathBuf::from("owned-prefetch.png"),
        MediaKind::Image,
        Time(1_000_000),
    );
    asset.width = 1;
    asset.height = 1;
    (service, asset, PreviewConsumer::new())
}

fn take(service: &MediaService) -> PendingFrame {
    service
        .shared
        .queue
        .lock()
        .unwrap()
        .take_pending_frame()
        .unwrap()
}

fn publish_deferred(
    service: &MediaService,
    job: &FrameJob,
    result: Result<Frame, MediaError>,
) -> bool {
    publish_result_with_admission(&service.shared, job, result, || {}, &mut 0, |_| {}, false)
        .unwrap()
}

#[test]
fn requested_picture_is_pollable_before_prefetch_finishes_and_seek_supersedes_its_guard() {
    let (service, asset, owner) = fixture();
    service
        .request_playback_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let PendingFrame { job, purpose } = take(&service);
    assert!(purpose == FramePurpose::Playback);
    assert!(publish_deferred(
        &service,
        &job,
        Ok(Frame::new(1, 1, vec![7, 0, 0, 255]).unwrap())
    ));
    let MediaEvent::FrameReady {
        time,
        consumer,
        frame,
        ..
    } = service.poll().unwrap()
    else {
        panic!("requested picture must be pollable while the bounded slice owns admission");
    };
    assert_eq!((time, consumer), (Time(10), Some(owner)));
    assert_eq!(&*frame.rgba, &[7, 0, 0, 255]);
    assert!(matches!(
        service.request_playback_preview(owner, &asset, Time(20), [1, 1]),
        Err(MediaError::Busy)
    ));
    assert!(read_ahead_current(&service.shared, &job, 0));
    service
        .request_preview(owner, &asset, Time(900_000), [1, 1])
        .unwrap();
    assert!(!read_ahead_current(&service.shared, &job, 0));
    service.shared.queue.lock().unwrap().finish_frame(&job);
    let PendingFrame { job: seek, purpose } = take(&service);
    assert!(purpose == FramePurpose::Interactive);
    assert_eq!(seek.time, Time(900_000));
    assert!(
        publish_result(
            &service.shared,
            &seek,
            Ok(Frame::new(1, 1, vec![9, 0, 0, 255]).unwrap())
        )
        .unwrap()
    );
    let MediaEvent::FrameReady { time, frame, .. } = service.poll().unwrap() else {
        panic!("seek picture");
    };
    assert_eq!(time, Time(900_000));
    assert_eq!(&*frame.rgba, &[9, 0, 0, 255]);
    assert!(service.shared.queue.lock().unwrap().in_flight.is_none());
}

#[test]
fn requested_playback_and_urgent_work_preempt_speculation() {
    let (service, asset, owner) = fixture();
    let other = PreviewConsumer::new();
    service
        .request_playback_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let job = take(&service).job;
    service
        .request_playback_preview(other, &asset, Time(20), [1, 1])
        .unwrap();
    assert!(!read_ahead_current(&service.shared, &job, 0));
    service.retire_preview(other);
    let before_hint = service.shared.queue.lock().unwrap().maintenance;
    assert!(read_ahead_current(&service.shared, &job, before_hint));
    service.request_poster(&asset, Time::ZERO, [1, 1]).unwrap();
    assert!(!read_ahead_current(&service.shared, &job, before_hint));
    service
        .shared
        .queue
        .lock()
        .unwrap()
        .frames
        .retain(|_, frame| frame.purpose != FramePurpose::Poster);
    assert!(read_ahead_current(&service.shared, &job, before_hint));
    service
        .set_preview_retention([Some(owner), None, None])
        .unwrap();
    assert!(!read_ahead_current(&service.shared, &job, before_hint));
    let maintenance = service.shared.queue.lock().unwrap().maintenance;
    assert!(read_ahead_current(&service.shared, &job, maintenance));
    service.retire_preview(owner);
    assert!(!read_ahead_current(&service.shared, &job, maintenance));
}

#[test]
fn error_and_stale_results_release_admission_without_a_speculative_slice() {
    let (service, asset, owner) = fixture();
    service
        .request_playback_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let job = take(&service).job;
    assert!(publish_deferred(
        &service,
        &job,
        Err(MediaError::Decode("owned failure".into()))
    ));
    assert!(matches!(service.poll(), Some(MediaEvent::Error { .. })));
    assert!(service.shared.queue.lock().unwrap().in_flight.is_none());
    service
        .request_playback_preview(owner, &asset, Time(20), [1, 1])
        .unwrap();
    let stale = take(&service).job;
    service
        .request_preview(owner, &asset, Time(30), [1, 1])
        .unwrap();
    assert!(!publish_deferred(
        &service,
        &stale,
        Ok(Frame::new(1, 1, vec![0, 0, 0, 255]).unwrap())
    ));
    assert!(service.poll().is_none());
    assert!(service.shared.queue.lock().unwrap().in_flight.is_none());
    let next = take(&service);
    assert!(next.purpose == FramePurpose::Interactive);
    assert_eq!(next.job.time, Time(30));
}
