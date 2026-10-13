//! Explicit preview ownership, independent of asset bounds and poster purpose.
use super::*;
use crate::Frame;

fn fixture() -> MediaService {
    MediaService {
        shared: Arc::new(Shared::default()),
        processes: Processes::default(),
    }
}

fn asset() -> Asset {
    Asset::new(
        PathBuf::from("owned-consumer.png"),
        MediaKind::Image,
        Time(1_000_000),
    )
}

fn take(service: &MediaService) -> FrameJob {
    service.shared.queue.lock().unwrap().take_frame().unwrap()
}

fn publish(service: &MediaService, job: &FrameJob, color: u8) -> bool {
    publish_result(
        &service.shared,
        job,
        Ok(Frame::new(1, 1, vec![color, 0, 0, 255]).unwrap()),
    )
    .unwrap()
}

fn result(
    service: &MediaService,
    consumer: Option<PreviewConsumer>,
    expected_time: Time,
    color: u8,
) {
    let MediaEvent::FrameReady {
        consumer: actual,
        time,
        frame,
        ..
    } = service.poll().unwrap()
    else {
        panic!("expected preview result");
    };
    assert_eq!(actual, consumer);
    assert_eq!(time, expected_time);
    assert_eq!(&*frame.rgba, &[color, 0, 0, 255]);
}

#[test]
fn equal_asset_and_bounds_consumers_keep_independent_latest_demand_and_unpolled_results() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    let b = PreviewConsumer::new();
    service
        .request_preview(a, &source, Time(100), [1, 1])
        .unwrap();
    service
        .request_preview(b, &source, Time(200), [1, 1])
        .unwrap();
    let first = take(&service);
    assert_eq!(first.lane, Lane::Consumer(a));
    assert!(publish(&service, &first, 1));
    let second = take(&service);
    assert_eq!(second.lane, Lane::Consumer(b));
    assert!(publish(&service, &second, 2));
    assert_eq!(service.shared.queue.lock().unwrap().events.len(), 2);
    result(&service, Some(a), Time(100), 1);
    result(&service, Some(b), Time(200), 2);
}

#[test]
fn one_consumer_supersedes_its_old_time_size_and_source_with_one_pending_lane() {
    let service = fixture();
    let a = PreviewConsumer::new();
    let old = asset();
    let new = asset();
    service
        .request_preview(a, &old, Time::ZERO, [1, 1])
        .unwrap();
    let captured = take(&service);
    service.request_preview(a, &new, Time(300), [2, 3]).unwrap();
    assert!(
        !service
            .shared
            .queue
            .lock()
            .unwrap()
            .frame_current(&captured)
    );
    assert!(!publish(&service, &captured, 1));
    assert!(service.poll().is_none());
    assert_eq!(service.shared.queue.lock().unwrap().frames.len(), 1);
    let replacement = take(&service);
    assert_eq!(
        (replacement.asset.id, replacement.size, replacement.time),
        (new.id, [2, 3], Time(300))
    );
    assert!(publish(&service, &replacement, 2));
    result(&service, Some(a), Time(300), 2);
}

#[test]
fn explicit_consumer_replacement_keeps_admission_rank_while_other_consumers_renew() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    let b = PreviewConsumer::new();
    service
        .request_preview(a, &source, Time(1), [1, 1])
        .unwrap();
    service
        .request_preview(b, &source, Time(2), [1, 1])
        .unwrap();
    service
        .request_preview(a, &source, Time(3), [2, 2])
        .unwrap();
    service
        .request_preview(b, &source, Time(4), [1, 1])
        .unwrap();
    let job = take(&service);
    assert_eq!(
        (job.lane, job.size, job.time),
        (Lane::Consumer(a), [2, 2], Time(3))
    );
}

#[test]
fn legacy_and_poster_methods_coalesce_without_replacing_equal_bounds_explicit_results() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    service
        .request_preview(a, &source, Time(1), [1, 1])
        .unwrap();
    let explicit = take(&service);
    assert!(publish(&service, &explicit, 1));
    service.request_poster(&source, Time(2), [1, 1]).unwrap();
    service.request_frame(&source, Time(3), [1, 1]).unwrap();
    assert_eq!(service.shared.queue.lock().unwrap().frames.len(), 1);
    let legacy = take(&service);
    assert!(matches!(legacy.lane, Lane::Legacy { .. }));
    assert_eq!(legacy.time, Time(3));
    assert!(publish(&service, &legacy, 2));
    result(&service, Some(a), Time(1), 1);
    result(&service, None, Time(3), 2);
}

#[test]
fn retiring_a_consumer_releases_its_pending_and_unpolled_work_and_rejects_captured_results() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    let b = PreviewConsumer::new();
    service
        .request_preview(a, &source, Time(1), [1, 1])
        .unwrap();
    assert!(publish(&service, &take(&service), 1));
    service
        .request_preview(b, &source, Time(2), [1, 1])
        .unwrap();
    assert!(publish(&service, &take(&service), 2));
    service
        .request_preview(a, &source, Time(3), [1, 1])
        .unwrap();
    let captured = take(&service);
    service
        .request_preview(a, &source, Time(4), [2, 2])
        .unwrap();
    service.retire_preview(a);
    assert!(!publish(&service, &captured, 3));
    let queue = service.shared.queue.lock().unwrap();
    assert!(queue.frames.is_empty() && queue.pending_order.is_empty());
    assert!(!queue.latest.contains_key(&Lane::Consumer(a)));
    assert_eq!(queue.event_bytes, 4);
    assert!(queue.in_flight.is_none());
    assert_eq!(queue.maintenance, 1);
    drop(queue);
    result(&service, Some(b), Time(2), 2);
    assert!(service.poll().is_none());
}

#[test]
fn cancellation_discards_all_consumer_results_and_exact_finish_tags_protect_new_work() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    service
        .request_preview(a, &source, Time(1), [1, 1])
        .unwrap();
    let old = take(&service);
    let generation = service.cancel();
    assert_ne!(generation, old.generation);
    service
        .request_preview(a, &source, Time(2), [1, 1])
        .unwrap();
    let next = take(&service);
    assert!(!publish(&service, &old, 1));
    assert!(service.shared.queue.lock().unwrap().in_flight == Some(next.identity()));
    assert!(publish(&service, &next, 2));
    result(&service, Some(a), Time(2), 2);
}

#[test]
fn explicit_consumers_obey_pending_capacity_and_can_replace_when_full() {
    let service = fixture();
    let source = asset();
    let consumers: Vec<_> = (0..32).map(|_| PreviewConsumer::new()).collect();
    for &consumer in &consumers {
        service
            .request_preview(consumer, &source, Time::ZERO, [1, 1])
            .unwrap();
    }
    assert!(matches!(
        service.request_preview(PreviewConsumer::new(), &source, Time::ZERO, [1, 1]),
        Err(MediaError::Busy)
    ));
    service
        .request_preview(consumers[0], &source, Time(9), [2, 3])
        .unwrap();
    let first = take(&service);
    assert_eq!(
        (first.lane, first.size, first.time),
        (Lane::Consumer(consumers[0]), [2, 3], Time(9))
    );
    service.retire_preview(consumers[1]);
    service
        .request_preview(PreviewConsumer::new(), &source, Time::ZERO, [1, 1])
        .unwrap();
    assert!(service.shared.queue.lock().unwrap().frames.len() <= 32);
}

#[test]
fn consumer_failures_keep_their_owner_and_bounds_and_retirement_discards_only_its_errors() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    let b = PreviewConsumer::new();
    for consumer in [a, b] {
        service
            .request_preview(consumer, &source, Time::ZERO, [2, 3])
            .unwrap();
        let job = take(&service);
        assert!(
            publish_result(
                &service.shared,
                &job,
                Err(MediaError::Decode("owned failure".into()))
            )
            .unwrap()
        );
    }
    service.retire_preview(a);
    let Some(MediaEvent::Error {
        asset_id,
        consumer,
        size,
        ..
    }) = service.poll()
    else {
        panic!("other consumer's error must be retained");
    };
    assert_eq!(
        (asset_id, consumer, size),
        (Some(source.id), Some(b), Some([2, 3]))
    );
    assert!(service.poll().is_none());
}

#[test]
fn publication_maintenance_reclaims_outside_the_queue_lock_and_rechecks_cancellation() {
    let service = fixture();
    let source = asset();
    let retired = PreviewConsumer::new();
    let current = PreviewConsumer::new();
    service
        .request_preview(retired, &source, Time::ZERO, [1, 1])
        .unwrap();
    let retired_job = take(&service);
    assert!(publish(&service, &retired_job, 1));
    service
        .request_preview(current, &source, Time::ZERO, [1, 1])
        .unwrap();
    let job = take(&service);
    service.retire_preview(retired);
    let mut observed = 0;
    let mut reclaimed = false;
    let result = publish_result_before_wait_with_maintenance(
        &service.shared,
        &job,
        Ok(Frame::new(1, 1, vec![0; 4]).unwrap()),
        || {},
        &mut observed,
        |snapshot| {
            assert!(
                service.shared.queue.try_lock().is_ok(),
                "decoder teardown must not hold the caller's queue mutex"
            );
            assert!(!snapshot.retained.contains(&Lane::Consumer(retired)));
            assert!(snapshot.retained.contains(&Lane::Consumer(current)));
            service.cancel();
            reclaimed = true;
        },
    )
    .unwrap();
    assert!(reclaimed);
    assert!(
        !result,
        "cancel during cleanup must invalidate the held result before publication"
    );
    assert!(service.poll().is_none());
}

#[test]
fn retiring_the_only_video_consumer_wakes_idle_worker_and_releases_retained_decoder_state() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("retirement.nut");
    let status = crate::process::tool_command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=256x128:rate=30",
            "-frames:v",
            "120",
            "-c:v",
            "ffv1",
            "-threads",
            "2",
            "-f",
            "nut",
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "owned retirement video could not be generated"
    );
    let mut source = Asset::new(path, MediaKind::Video, Time(4_000_000));
    source.width = 256;
    source.height = 128;
    source.frame_rate = Some(editor_core::FrameRate::default());
    let service = MediaService::new(|| {}).unwrap();
    let consumer = PreviewConsumer::new();
    service
        .request_preview(consumer, &source, Time::ZERO, [256, 128])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = service.poll() {
            let MediaEvent::FrameReady {
                consumer: returned, ..
            } = event
            else {
                panic!("expected first frame, received {event:?}");
            };
            assert_eq!(returned, Some(consumer));
            break;
        }
        assert!(Instant::now() < deadline, "owned first frame timed out");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.shared.queue.lock().unwrap().frames.is_empty());
    assert!(
        service.processes.retained_monitors() > 0,
        "decoder state must be retained before retirement"
    );
    service.retire_preview(consumer);
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.processes.retained_monitors() != 0 {
        assert!(
            Instant::now() < deadline,
            "idle worker did not release retained decoder monitors after retirement"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.poll().is_none());
}

#[test]
fn retiring_a_decoder_consumer_reclaims_it_while_another_result_is_byte_blocked() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    // Drop the live service and wait only for its owned monitors before the
    // fixture directory is removed, including while an assertion unwinds.
    struct OwnedService {
        service: Option<MediaService>,
        processes: Processes,
    }
    impl Drop for OwnedService {
        fn drop(&mut self) {
            drop(self.service.take());
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.processes.retained_monitors() != 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let video = temp.path().join("retired.nut");
    let status = crate::process::tool_command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=256x128:rate=30",
            "-frames:v",
            "120",
            "-c:v",
            "ffv1",
            "-threads",
            "2",
            "-f",
            "nut",
        ])
        .arg(&video)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let mut source = Asset::new(video, MediaKind::Video, Time(4_000_000));
    source.width = 256;
    source.height = 128;
    source.frame_rate = Some(editor_core::FrameRate::default());
    let media = MediaService::new(|| {}).unwrap();
    let fixture = OwnedService {
        processes: media.processes.clone(),
        service: Some(media),
    };
    let service = fixture.service.as_ref().unwrap();
    let owner = PreviewConsumer::new();
    service
        .request_preview(owner, &source, Time::ZERO, [256, 128])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(MediaEvent::FrameReady {
            consumer: Some(id), ..
        }) = service.poll()
        {
            assert_eq!(id, owner);
            break;
        }
        assert!(Instant::now() < deadline, "initial owned video timed out");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.processes.retained_monitors() > 0);
    let path = temp.path().join("large.png");
    image::RgbaImage::from_pixel(1920, 1920, image::Rgba([0, 0, 0, 255]))
        .save(&path)
        .unwrap();
    let mut images = Vec::new();
    for _ in 0..5 {
        let mut image = Asset::new(path.clone(), MediaKind::Image, Time(1_000_000));
        image.width = 1920;
        image.height = 1920;
        images.push(image);
    }
    for (index, image) in images[..4].iter().enumerate() {
        service
            .request_frame(image, Time::ZERO, [1920, 1920])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if service.shared.queue.lock().unwrap().events.len() == index + 1 {
                break;
            }
            assert!(Instant::now() < deadline, "owned mailbox filling timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }
    let blocked = PreviewConsumer::new();
    service
        .request_preview(blocked, &images[4], Time::ZERO, [1920, 1920])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if service
            .shared
            .queue
            .lock()
            .unwrap()
            .in_flight
            .is_some_and(|tag| tag.lane == Lane::Consumer(blocked))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "unrelated consumer did not begin its owned job"
        );
        thread::sleep(Duration::from_millis(1));
    }
    // The fifth 14,745,600-byte result cannot fit with the four unpolled ones.
    service.retire_preview(owner);
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.processes.retained_monitors() != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        service.processes.retained_monitors(),
        0,
        "retirement must reclaim A even while unrelated B waits for consumer mailbox space"
    );
    let queue = service.shared.queue.lock().unwrap();
    assert_eq!(
        queue.events.len(),
        4,
        "retirement must preserve the four unrelated outcomes"
    );
    assert_eq!(queue.event_bytes, 4 * 1920 * 1920 * 4);
    assert!(
        queue
            .in_flight
            .is_some_and(|tag| tag.lane == Lane::Consumer(blocked))
    );
}

// PREPARED ONLY — no formatter, compiler or runtime executed by the author.
// Append to crates/editor-media/src/service/consumer_tests.rs after root adds:
//
//   MediaService::set_preview_retention([Option<PreviewConsumer>; 3])
//       -> Result<(), MediaError>
//   Queue.preview_retention: [Option<PreviewConsumer>; 3]
//   Queue::retained_lanes() -> Vec<Lane>
//   PreviewMaintenance { retained: Vec<Lane>, protected: [Option<PreviewConsumer>; 3] }
//
// publish_result_before_wait_with_maintenance must give its off-mutex callback
// &PreviewMaintenance. Existing fixture/asset/take/publish/result helpers and
// imports from consumer_tests.rs are reused. Missing API compilation is not RED
// evidence; these lifecycle controls are to execute after implementation.
//
// Two limitations needing root review/plumbing:
// 1. The normalized no-op checks its epoch and queue state, not a count of raw
//    Condvar notifications. Verify the setter's unchanged-membership return is
//    before ready.notify_all(), or add a production notification observation
//    seam if exact no-wake behavior needs a regression.
// 2. The cancellation race below does not run preview_worker/DecoderPool. Add a
//    control at the production generation+maintenance application seam: cancel,
//    install new-generation hints before worker dispatch, capture that coherent
//    snapshot, clear old-generation decoders, THEN apply snapshot.protected even
//    when generation changed. Do not consume the epoch through an else-if that
//    skips configuring fresh hints. With a real retained hinted consumer plus
//    competing transient consumers, prove the hinted continuous pipe survives
//    using existing decoder child/start/pixel observations. Queue hints alone
//    cannot establish decoder configuration, process starts or throughput.

fn retention_hint_members(
    pins: &[Option<PreviewConsumer>; 3],
) -> std::collections::BTreeSet<PreviewConsumer> {
    pins.iter().flatten().copied().collect()
}

#[test]
fn preview_retention_normalizes_membership_and_replaces_hints_without_creating_demand() {
    let service = fixture();
    let a = PreviewConsumer::new();
    let b = PreviewConsumer::new();
    let initial = service.shared.queue.lock().unwrap().maintenance;
    service
        .set_preview_retention([Some(a), Some(a), None])
        .unwrap();
    let first_epoch = {
        let queue = service.shared.queue.lock().unwrap();
        assert_ne!(queue.maintenance, initial);
        assert_eq!(
            retention_hint_members(&queue.preview_retention),
            std::collections::BTreeSet::from([a])
        );
        assert_eq!(queue.preview_retention.iter().flatten().count(), 1);
        assert!(queue.latest.is_empty() && queue.frames.is_empty());
        assert!(queue.in_flight.is_none() && queue.events.is_empty());
        assert_eq!(queue.sequence, 0);
        queue.maintenance
    };
    service
        .set_preview_retention([None, Some(a), Some(a)])
        .unwrap();
    assert_eq!(
        service.shared.queue.lock().unwrap().maintenance,
        first_epoch
    );

    service
        .set_preview_retention([Some(b), Some(a), None])
        .unwrap();
    let changed_epoch = service.shared.queue.lock().unwrap().maintenance;
    assert_ne!(changed_epoch, first_epoch);
    service
        .set_preview_retention([Some(a), Some(b), Some(a)])
        .unwrap();
    {
        let queue = service.shared.queue.lock().unwrap();
        assert_eq!(queue.maintenance, changed_epoch);
        assert_eq!(
            retention_hint_members(&queue.preview_retention),
            std::collections::BTreeSet::from([a, b])
        );
        assert_eq!(queue.preview_retention.iter().flatten().count(), 2);
        assert!(queue.latest.is_empty() && queue.frames.is_empty());
    }

    service
        .set_preview_retention([Some(b), None, None])
        .unwrap();
    {
        let queue = service.shared.queue.lock().unwrap();
        assert_eq!(
            retention_hint_members(&queue.preview_retention),
            std::collections::BTreeSet::from([b])
        );
        assert!(!queue.retained_lanes().contains(&Lane::Consumer(a)));
        assert!(queue.retained_lanes().contains(&Lane::Consumer(b)));
    }
    service.set_preview_retention([None; 3]).unwrap();
    let empty_epoch = service.shared.queue.lock().unwrap().maintenance;
    service.set_preview_retention([None; 3]).unwrap();
    let queue = service.shared.queue.lock().unwrap();
    assert_eq!(queue.maintenance, empty_epoch);
    assert_eq!(queue.preview_retention, [None; 3]);
    assert!(queue.retained_lanes().is_empty());
}

#[test]
fn preview_retention_does_not_reserve_admission_or_make_a_superseded_request_current() {
    let service = fixture();
    let source = asset();
    let consumers: Vec<_> = (0..32).map(|_| PreviewConsumer::new()).collect();
    for &consumer in &consumers {
        service
            .request_preview(consumer, &source, Time::ZERO, [1, 1])
            .unwrap();
    }
    let unknown = PreviewConsumer::new();
    service
        .set_preview_retention([Some(unknown), Some(consumers[0]), None])
        .unwrap();
    {
        let queue = service.shared.queue.lock().unwrap();
        assert_eq!(queue.frames.len(), 32);
        assert_eq!(queue.latest.len(), 32);
        assert!(!queue.latest.contains_key(&Lane::Consumer(unknown)));
    }
    assert!(matches!(
        service.request_preview(unknown, &source, Time(1), [1, 1]),
        Err(MediaError::Busy)
    ));
    service
        .request_preview(consumers[0], &source, Time(2), [1, 1])
        .unwrap();
    let old = take(&service);
    assert_eq!(old.lane, Lane::Consumer(consumers[0]));
    assert_eq!(old.time, Time(2));
    service
        .request_preview(consumers[0], &source, Time(3), [2, 3])
        .unwrap();
    assert!(!service.shared.queue.lock().unwrap().frame_current(&old));
    assert!(!publish(&service, &old, 1));
    assert!(service.poll().is_none());
    let queue = service.shared.queue.lock().unwrap();
    assert_eq!(queue.frames.len(), 32);
    assert_eq!(queue.frames[&Lane::Consumer(consumers[0])].job.size, [2, 3]);
    assert_eq!(
        queue.frames[&Lane::Consumer(consumers[0])].job.time,
        Time(3)
    );
    assert!(!queue.latest.contains_key(&Lane::Consumer(unknown)));
}

#[test]
fn preview_retention_protects_completed_pins_through_history_churn_and_protected_work() {
    let service = fixture();
    let source = asset();
    let pins = [
        PreviewConsumer::new(),
        PreviewConsumer::new(),
        PreviewConsumer::new(),
    ];
    service.set_preview_retention(pins.map(Some)).unwrap();
    let mut pinned_jobs = Vec::new();
    for (index, &consumer) in pins.iter().enumerate() {
        service
            .request_preview(consumer, &source, Time(index as i64), [1, 1])
            .unwrap();
        let job = take(&service);
        assert!(publish(&service, &job, (index + 1) as u8));
        result(&service, Some(consumer), job.time, (index + 1) as u8);
        pinned_jobs.push(job);
    }
    // Real public request -> single-worker selection -> publication -> poll.
    for index in 0..65 {
        let completed = asset();
        let time = Time(100 + index);
        service.request_frame(&completed, time, [1, 1]).unwrap();
        let job = take(&service);
        assert_eq!(job.asset.id, completed.id);
        assert!(publish(&service, &job, 7));
        result(&service, None, time, 7);
        let queue = service.shared.queue.lock().unwrap();
        assert!(queue.latest.len() <= 64);
        for pin in &pinned_jobs {
            assert_eq!(queue.latest.get(&pin.lane), Some(&pin.sequence));
        }
    }
    let in_flight = PreviewConsumer::new();
    let pending = PreviewConsumer::new();
    service
        .request_preview(in_flight, &source, Time(17), [1, 1])
        .unwrap();
    let held = take(&service);
    service
        .request_preview(pending, &source, Time(19), [1, 1])
        .unwrap();
    {
        let mut queue = service.shared.queue.lock().unwrap();
        // Explicitly synthetic COMPLETED-history saturation at the production
        // pruning seam, as in delivery_tests. A single real worker cannot
        // complete other jobs while preserving this captured in-flight job.
        // Never select another job or replace its exact in-flight identity.
        for _ in 0..65 {
            let completed = Lane::Legacy {
                asset_id: asset().id,
                size: [1, 1],
            };
            queue.sequence = queue.sequence.wrapping_add(1);
            let sequence = queue.sequence;
            queue.latest.insert(completed, sequence);
            queue.prune_latest();
            assert!(queue.latest.len() <= 64);
            for pin in &pinned_jobs {
                assert_eq!(queue.latest.get(&pin.lane), Some(&pin.sequence));
            }
            assert!(queue.frame_current(&held));
            assert!(queue.in_flight == Some(held.identity()));
            assert!(queue.frames.contains_key(&Lane::Consumer(pending)));
        }
    }
    assert!(publish(&service, &held, 8));
    result(&service, Some(in_flight), Time(17), 8);
    let next = take(&service);
    assert_eq!(next.lane, Lane::Consumer(pending));
    assert!(publish(&service, &next, 9));
    result(&service, Some(pending), Time(19), 9);
}

#[test]
fn preview_retention_unions_pruned_tags_without_recreating_currentness_or_metadata() {
    let service = fixture();
    let source = asset();
    let old = PreviewConsumer::new();
    service
        .request_preview(old, &source, Time::ZERO, [1, 1])
        .unwrap();
    let completed_old = take(&service);
    assert!(publish(&service, &completed_old, 1));
    result(&service, Some(old), Time::ZERO, 1);
    for _ in 0..65 {
        let other = asset();
        service.request_frame(&other, Time::ZERO, [1, 1]).unwrap();
        let job = take(&service);
        assert!(publish(&service, &job, 2));
        result(&service, None, Time::ZERO, 2);
    }
    let (latest_before, sequence_before, generation_before) = {
        let queue = service.shared.queue.lock().unwrap();
        assert_eq!(queue.latest.len(), 64);
        assert!(!queue.latest.contains_key(&Lane::Consumer(old)));
        (queue.latest.clone(), queue.sequence, queue.generation)
    };
    let absent = [PreviewConsumer::new(), PreviewConsumer::new()];
    service
        .set_preview_retention([Some(old), Some(absent[0]), Some(absent[1])])
        .unwrap();
    {
        let queue = service.shared.queue.lock().unwrap();
        assert_eq!(queue.latest, latest_before);
        assert_eq!(queue.sequence, sequence_before);
        assert_eq!(queue.generation, generation_before);
        assert!(queue.frames.is_empty() && queue.events.is_empty());
        assert!(!queue.frame_current(&completed_old));
        let retained = queue.retained_lanes();
        assert_eq!(retained.len(), 67);
        assert_eq!(
            retained
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            67
        );
        for consumer in [old, absent[0], absent[1]] {
            assert!(retained.contains(&Lane::Consumer(consumer)));
            assert!(!queue.latest.contains_key(&Lane::Consumer(consumer)));
        }
    }
    assert!(!publish(&service, &completed_old, 3));
    assert!(service.poll().is_none());
    // This proves the queue union for a previously completed/tag-pruned owner;
    // it does not claim this workerless fixture still owns a warm decoder pipe.
}

#[test]
fn preview_retirement_and_project_cancel_clear_only_their_matching_retention_hints() {
    let service = fixture();
    let source = asset();
    let a = PreviewConsumer::new();
    let b = PreviewConsumer::new();
    let c = PreviewConsumer::new();
    service
        .set_preview_retention([Some(a), Some(b), Some(c)])
        .unwrap();
    service
        .request_preview(a, &source, Time::ZERO, [1, 1])
        .unwrap();
    let a_job = take(&service);
    assert!(publish(&service, &a_job, 1));
    service
        .request_preview(b, &source, Time(2), [1, 1])
        .unwrap();
    let b_job = take(&service);
    let before_retire = service.shared.queue.lock().unwrap().maintenance;
    service.retire_preview(a);
    {
        let queue = service.shared.queue.lock().unwrap();
        assert_ne!(queue.maintenance, before_retire);
        assert_eq!(
            retention_hint_members(&queue.preview_retention),
            std::collections::BTreeSet::from([b, c])
        );
        assert!(!queue.retained_lanes().contains(&Lane::Consumer(a)));
        assert!(!queue.latest.contains_key(&Lane::Consumer(a)));
        assert!(queue.events.is_empty());
        assert!(queue.frame_current(&b_job));
        assert!(queue.in_flight == Some(b_job.identity()));
    }
    let old_generation = b_job.generation;
    assert_ne!(service.cancel(), old_generation);
    {
        let queue = service.shared.queue.lock().unwrap();
        assert_eq!(queue.preview_retention, [None; 3]);
        assert!(queue.retained_lanes().is_empty() && queue.latest.is_empty());
        assert!(queue.frames.is_empty() && queue.events.is_empty());
        assert!(queue.in_flight.is_none());
    }
    assert!(!publish(&service, &b_job, 2));
    service
        .set_preview_retention([Some(c), None, None])
        .unwrap();
    let queue = service.shared.queue.lock().unwrap();
    assert_eq!(
        retention_hint_members(&queue.preview_retention),
        std::collections::BTreeSet::from([c])
    );
    assert!(queue.latest.is_empty());
    assert!(!queue.frame_current(&b_job));
}

#[test]
fn a_closed_media_service_rejects_preview_retention_changes() {
    let service = fixture();
    let pin = PreviewConsumer::new();
    service
        .set_preview_retention([Some(pin), None, None])
        .unwrap();
    let (before, before_epoch) = {
        let mut queue = service.shared.queue.lock().unwrap();
        queue.stopped = true; // Production Drop's closed-state seam.
        (queue.preview_retention, queue.maintenance)
    };
    assert!(matches!(
        service.set_preview_retention([None; 3]),
        Err(MediaError::Closed)
    ));
    let queue = service.shared.queue.lock().unwrap();
    assert_eq!(queue.preview_retention, before);
    assert_eq!(queue.maintenance, before_epoch);
    assert!(queue.latest.is_empty());
}

#[derive(Debug)]
enum RetentionPublicationProgress {
    Waiting,
    Maintenance {
        protected: [Option<PreviewConsumer>; 3],
        retained: Vec<Lane>,
        queue_unlocked: bool,
    },
    Finished(Result<bool, MediaError>),
}

// Exactly one production publication call, no import/decoder/native workers.
// Test-only rendezvous pauses the actual off-mutex maintenance callback while
// the main thread uses real public setters/cancel/poll. All waits are finite.
struct RetentionHintPublisher {
    service: Option<MediaService>,
    thread: Option<std::thread::JoinHandle<()>>,
    progress: std::sync::mpsc::Receiver<RetentionPublicationProgress>,
    resume: std::sync::mpsc::Sender<()>,
}

impl RetentionHintPublisher {
    fn new(service: MediaService, job: FrameJob, frame: Frame) -> Self {
        let shared = Arc::clone(&service.shared);
        // Ignore setup epochs: the first observed callback must belong to the
        // setter called only after we observe the real before-wait seam.
        let mut observed = shared.queue.lock().unwrap().maintenance;
        let (sender, progress) = std::sync::mpsc::channel();
        let waiting = sender.clone();
        let (resume, resumed) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let outcome = publish_result_before_wait_with_maintenance(
                &shared,
                &job,
                Ok(frame),
                || {
                    let _ = waiting.send(RetentionPublicationProgress::Waiting);
                },
                &mut observed,
                |snapshot| {
                    let queue_unlocked = shared.queue.try_lock().is_ok();
                    let _ = sender.send(RetentionPublicationProgress::Maintenance {
                        protected: snapshot.protected,
                        retained: snapshot.retained.clone(),
                        queue_unlocked,
                    });
                    resumed
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .expect("owned maintenance callback was not resumed");
                },
            );
            let _ = sender.send(RetentionPublicationProgress::Finished(outcome));
        });
        Self {
            service: Some(service),
            thread: Some(thread),
            progress,
            resume,
        }
    }

    fn service(&self) -> &MediaService {
        self.service.as_ref().unwrap()
    }

    fn next(&self) -> RetentionPublicationProgress {
        self.progress
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("owned publisher observation timed out")
    }

    fn expect_wait(&self) {
        assert!(matches!(self.next(), RetentionPublicationProgress::Waiting));
        // Observes the before-wait hook, not an OS-level parked thread.
        // Setters take that same mutex before notifying, avoiding lost wakes.
    }

    fn expect_snapshot(
        &self,
        expected: PreviewConsumer,
        replaced: PreviewConsumer,
        held: &FrameJob,
    ) {
        let RetentionPublicationProgress::Maintenance {
            protected,
            retained,
            queue_unlocked,
        } = self.next()
        else {
            panic!("expected the changed hint's maintenance snapshot");
        };
        assert!(queue_unlocked);
        assert_eq!(
            retention_hint_members(&protected),
            std::collections::BTreeSet::from([expected])
        );
        assert!(retained.contains(&Lane::Consumer(expected)));
        assert!(!retained.contains(&Lane::Consumer(replaced)));
        assert!(retained.contains(&held.lane));
        assert!(retained.len() <= 67);
        assert_eq!(
            retained.len(),
            retained
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
        let queue = self.service().shared.queue.lock().unwrap();
        assert!(queue.frame_current(held));
        assert!(queue.in_flight == Some(held.identity()));
        assert_eq!(
            retained
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            queue.retained_lanes().into_iter().collect()
        );
        assert_eq!(
            retention_hint_members(&protected),
            retention_hint_members(&queue.preview_retention)
        );
    }

    fn finish(&mut self) -> Result<bool, MediaError> {
        let outcome = loop {
            match self.next() {
                RetentionPublicationProgress::Finished(outcome) => break outcome,
                RetentionPublicationProgress::Waiting => {}
                other => panic!("unexpected extra maintenance pass: {other:?}"),
            }
        };
        self.thread
            .take()
            .unwrap()
            .join()
            .expect("owned publisher panicked");
        outcome
    }
}

impl Drop for RetentionHintPublisher {
    fn drop(&mut self) {
        // Production service Drop marks stopped and notifies the condvar.
        // That alone cannot release the test's callback rendezvous, so release
        // it too before joining, including during assertion unwinding.
        drop(self.service.take());
        let _ = self.resume.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn count_blocked_publication_reapplies_coherent_hint_changes_without_losing_results() {
    let service = fixture();
    let initial = PreviewConsumer::new();
    let first_change = PreviewConsumer::new();
    let second_change = PreviewConsumer::new();
    service
        .set_preview_retention([Some(initial), None, None])
        .unwrap();
    let mut sources = Vec::new();
    for index in 0..64 {
        let source = asset();
        let time = Time(index);
        service.request_frame(&source, time, [1, 1]).unwrap();
        let job = take(&service);
        assert_eq!(job.asset.id, source.id);
        assert!(publish(&service, &job, (index + 1) as u8));
        sources.push(source);
    }
    let owner = PreviewConsumer::new();
    let held_source = asset();
    service
        .request_preview(owner, &held_source, Time(99), [1, 1])
        .unwrap();
    let held = take(&service);
    let mut publisher = RetentionHintPublisher::new(
        service,
        held.clone(),
        Frame::new(1, 1, vec![99, 0, 0, 255]).unwrap(),
    );
    publisher.expect_wait();
    publisher
        .service()
        .set_preview_retention([Some(first_change), None, None])
        .unwrap();
    publisher.expect_snapshot(first_change, initial, &held);

    // Change the real queue while the first snapshot's callback is still
    // outside the mutex; the next pass must pair NEW retained/protected values.
    publisher
        .service()
        .set_preview_retention([Some(second_change), None, None])
        .unwrap();
    publisher.resume.send(()).unwrap();
    publisher.expect_snapshot(second_change, first_change, &held);
    publisher.resume.send(()).unwrap();
    publisher.expect_wait();
    {
        let queue = publisher.service().shared.queue.lock().unwrap();
        assert_eq!(queue.events.len(), 64);
        assert_eq!(queue.event_bytes, 64 * 4);
        assert_eq!(
            queue
                .events
                .iter()
                .filter_map(|event| match event {
                    MediaEvent::FrameReady { asset_id, .. } => Some(*asset_id),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            sources.iter().map(|source| source.id).collect::<Vec<_>>()
        );
    }
    result(publisher.service(), None, Time::ZERO, 1);
    assert!(publisher.finish().unwrap());
    {
        let queue = publisher.service().shared.queue.lock().unwrap();
        assert_eq!(queue.events.len(), 64);
        assert_eq!(queue.event_bytes, 64 * 4);
        assert!(queue.in_flight.is_none());
        let ids: Vec<_> = queue
            .events
            .iter()
            .filter_map(|event| match event {
                MediaEvent::FrameReady { asset_id, .. } => Some(*asset_id),
                _ => None,
            })
            .collect();
        let mut expected: Vec<_> = sources[1..].iter().map(|source| source.id).collect();
        expected.push(held_source.id);
        assert_eq!(ids, expected);
    }
    for index in 1..64 {
        result(publisher.service(), None, Time(index), (index + 1) as u8);
    }
    result(publisher.service(), Some(owner), Time(99), 99);
    assert!(publisher.service().poll().is_none());
}

#[test]
fn byte_blocked_publication_rechecks_cancel_and_preserves_new_generation_hints() {
    let service = fixture();
    let initial = PreviewConsumer::new();
    let changed = PreviewConsumer::new();
    let next_generation_pin = PreviewConsumer::new();
    service
        .set_preview_retention([Some(initial), None, None])
        .unwrap();
    let size = [1920, 1920];
    let bytes = frame_bytes(size[0], size[1]).unwrap();
    let frame = Frame::new(size[0], size[1], vec![7; bytes]).unwrap();
    assert!(4 * bytes <= crate::FRAME_CACHE_BYTES && 5 * bytes > crate::FRAME_CACHE_BYTES);
    for _ in 0..4 {
        let source = asset();
        service.request_frame(&source, Time::ZERO, size).unwrap();
        let job = take(&service);
        assert!(publish_result(&service.shared, &job, Ok(frame.clone())).unwrap());
    }
    let owner = PreviewConsumer::new();
    let source = asset();
    service
        .request_preview(owner, &source, Time(99), size)
        .unwrap();
    let held = take(&service);
    let mut publisher = RetentionHintPublisher::new(service, held.clone(), frame);
    publisher.expect_wait();
    publisher
        .service()
        .set_preview_retention([Some(changed), None, None])
        .unwrap();
    publisher.expect_snapshot(changed, initial, &held);
    {
        let queue = publisher.service().shared.queue.lock().unwrap();
        assert_eq!(queue.events.len(), 4);
        assert_eq!(queue.event_bytes, 4 * bytes);
    }
    assert_ne!(publisher.service().cancel(), held.generation);
    publisher
        .service()
        .set_preview_retention([Some(next_generation_pin), None, None])
        .unwrap();
    publisher.resume.send(()).unwrap();
    assert!(!publisher.finish().unwrap());
    let queue = publisher.service().shared.queue.lock().unwrap();
    assert_eq!(
        retention_hint_members(&queue.preview_retention),
        std::collections::BTreeSet::from([next_generation_pin])
    );
    assert_eq!(
        queue.retained_lanes(),
        vec![Lane::Consumer(next_generation_pin)]
    );
    assert!(queue.latest.is_empty() && queue.frames.is_empty() && queue.events.is_empty());
    assert_eq!(queue.event_bytes, 0);
    assert!(queue.in_flight.is_none());
    assert!(!queue.frame_current(&held));
    // Fresh hint installation survived the OLD publisher; the header's worker
    // generation-application control is still required for actual decoder pins.
}

#[test]
fn the_real_worker_applies_new_generation_hints_and_keeps_a_newly_hinted_pruned_pipe() {
    use std::{
        io::Write,
        process::{Child, Stdio},
        sync::{
            Condvar,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };
    struct Encoder(Child);
    impl Drop for Encoder {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct OwnedService {
        service: Option<MediaService>,
        processes: Processes,
        release: Arc<(Mutex<bool>, Condvar)>,
    }
    impl Drop for OwnedService {
        fn drop(&mut self) {
            *self.release.0.lock().unwrap() = true;
            self.release.1.notify_all();
            drop(self.service.take());
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.processes.retained_monitors() != 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    // Lossless independently generated inputs distinguish every asset and time.
    let directory = tempfile::tempdir().unwrap();
    let rate = editor_core::FrameRate::default();
    let mut sources = Vec::new();
    let mut expected = Vec::new();
    for source_index in 0..6 {
        let pixels: Vec<Vec<u8>> = (0..4)
            .map(|frame| [source_index * 31, frame * 47, 123, 255].repeat(64 * 32))
            .collect();
        let raw_path = directory.path().join(format!("source-{source_index}.rgba"));
        let mut raw = std::fs::File::create(&raw_path).unwrap();
        for frame in &pixels {
            raw.write_all(frame).unwrap();
        }
        drop(raw);
        let path = directory.path().join(format!("source-{source_index}.nut"));
        let mut command = crate::process::tool_command("ffmpeg");
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
            .arg(&raw_path)
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
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut encoder = Encoder(command.spawn().expect("owned lossless worker fixture"));
        assert!(encoder.0.wait().unwrap().success());
        drop(encoder);
        let mut source = Asset::new(path, MediaKind::Video, rate.frame_time(4));
        source.width = 64;
        source.height = 32;
        source.frame_rate = Some(rate);
        source.file_size = std::fs::metadata(&source.path).unwrap().len();
        sources.push(source);
        expected.push(pixels);
    }

    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_release = Arc::clone(&release);
    let wakes = AtomicUsize::new(0);
    let (paused, observed_pause) = mpsc::channel();
    let service = MediaService::new(move || {
        if wakes.fetch_add(1, Ordering::AcqRel) == 0 {
            // The real preview worker is parked after its first publication,
            // with old-generation decoder state still owned by that worker.
            let _ = paused.send(());
            let released = worker_release.0.lock().unwrap();
            let (released, timeout) = worker_release
                .1
                .wait_timeout_while(released, Duration::from_secs(10), |released| !*released)
                .unwrap();
            assert!(
                *released && !timeout.timed_out(),
                "owned worker callback was not released"
            );
        }
    })
    .unwrap();
    let fixture = OwnedService {
        processes: service.processes.clone(),
        service: Some(service),
        release: Arc::clone(&release),
    };
    let service = fixture.service.as_ref().unwrap();
    let old = PreviewConsumer::new();
    service
        .set_preview_retention([Some(old), None, None])
        .unwrap();
    service
        .request_preview(old, &sources[5], Time::ZERO, [64, 32])
        .unwrap();
    observed_pause
        .recv_timeout(Duration::from_secs(5))
        .expect("real preview worker paused");
    let Some(MediaEvent::FrameReady {
        consumer, frame, ..
    }) = service.poll()
    else {
        panic!("old generation did not publish the exact first frame");
    };
    assert_eq!(consumer, Some(old));
    assert_eq!(frame.rgba.as_ref(), expected[5][0].as_slice());
    assert_eq!(service.processes.started_tools(), 1);

    let next_generation = service.cancel();
    let consumers: [PreviewConsumer; 5] = std::array::from_fn(|_| PreviewConsumer::new());
    service
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])])
        .unwrap();
    *release.0.lock().unwrap() = true;
    release.1.notify_all();

    for frame_index in [0, 1] {
        for (index, consumer) in consumers.iter().copied().enumerate() {
            let time = rate.frame_time(frame_index as i64);
            service
                .request_preview(consumer, &sources[index], time, [64, 32])
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let event = loop {
                if let Some(event) = service.poll() {
                    break event;
                }
                assert!(Instant::now() < deadline, "new-generation frame timed out");
                thread::sleep(Duration::from_millis(1));
            };
            let MediaEvent::FrameReady {
                generation,
                asset_id,
                consumer: returned,
                time: returned_time,
                size,
                frame,
            } = event
            else {
                panic!("expected exact new-generation frame, got {event:?}");
            };
            assert_eq!(
                (generation, asset_id, returned, returned_time, size),
                (
                    next_generation,
                    sources[index].id,
                    Some(consumer),
                    time,
                    [64, 32]
                )
            );
            assert_eq!([frame.width, frame.height], [64, 32]);
            assert_eq!(frame.rgba.as_ref(), expected[index][frame_index].as_slice());
        }
        assert_eq!(
            service.processes.started_tools(),
            6,
            "one old start, then five continuous new-generation pipes"
        );
    }
    service.set_preview_retention([None; 3]).unwrap();
    for consumer in consumers[1..].iter().copied() {
        service.retire_preview(consumer);
    }

    // Completed image jobs churn request history without displacing the one
    // remaining warm video pipe. The old consumer is no longer protected.
    let image = directory.path().join("history.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 19, 29, 255]))
        .save(&image)
        .unwrap();
    let wait = || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = service.poll() {
                return event;
            }
            assert!(Instant::now() < deadline, "owned history frame timed out");
            thread::sleep(Duration::from_millis(1));
        }
    };
    let still = || {
        let mut asset = Asset::new(image.clone(), MediaKind::Image, Time(1_000_000));
        asset.width = 1;
        asset.height = 1;
        asset
    };
    for _ in 0..65 {
        let asset = still();
        service.request_frame(&asset, Time::ZERO, [1, 1]).unwrap();
        let MediaEvent::FrameReady {
            asset_id, frame, ..
        } = wait()
        else {
            panic!("expected exact completed image history frame");
        };
        assert_eq!(asset_id, asset.id);
        assert_eq!(frame.rgba.as_ref(), &[9, 19, 29, 255]);
    }
    assert!(
        !service
            .shared
            .queue
            .lock()
            .unwrap()
            .latest
            .contains_key(&Lane::Consumer(consumers[0]))
    );
    assert_eq!(service.processes.started_tools(), 6);

    service
        .set_preview_retention([Some(consumers[0]), None, None])
        .unwrap();
    // Await a different completion to force the real maintenance path before
    // renewing the pruned consumer; renewal must not hide a missing allowlist.
    let asset = still();
    service.request_frame(&asset, Time::ZERO, [1, 1]).unwrap();
    let MediaEvent::FrameReady {
        asset_id, frame, ..
    } = wait()
    else {
        panic!("expected maintenance fence image frame");
    };
    assert_eq!(asset_id, asset.id);
    assert_eq!(frame.rgba.as_ref(), &[9, 19, 29, 255]);
    assert!(
        !service
            .shared
            .queue
            .lock()
            .unwrap()
            .latest
            .contains_key(&Lane::Consumer(consumers[0]))
    );
    let time = rate.frame_time(2);
    service
        .request_preview(consumers[0], &sources[0], time, [64, 32])
        .unwrap();
    let MediaEvent::FrameReady {
        consumer,
        time: returned_time,
        frame,
        ..
    } = wait()
    else {
        panic!("expected newly hinted warm consumer's next frame");
    };
    assert_eq!((consumer, returned_time), (Some(consumers[0]), time));
    assert_eq!([frame.width, frame.height], [64, 32]);
    assert_eq!(frame.rgba.as_ref(), expected[0][2].as_slice());
    assert_eq!(
        service.processes.started_tools(),
        6,
        "maintenance keeps an already warm newly hinted pipe with a pruned request tag"
    );
    service.retire_preview(consumers[0]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.processes.retained_monitors() != 0 {
        assert!(
            Instant::now() < deadline,
            "owned decoder monitors did not retire"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.poll().is_none());
}

/// A transition on each of three stacked video tracks, played through the
/// real worker: six consumers ask for their next frame every turn of the
/// application, one more than the decoder bank holds and three more than it
/// protects.
///
/// A consumer whose frame is not decoded yet is told `Busy` and asks again
/// the next turn, for the frame the clock is then on; none waits for ever,
/// and each is published frames of its own in order, never another's, and
/// ends on the frame the clock stopped at.
#[test]
fn six_video_consumers_of_a_stacked_transition_are_refused_in_turn_and_all_served() {
    use std::{
        io::Write,
        process::Stdio,
        time::{Duration, Instant},
    };
    const SIZE: [u32; 2] = [64, 32];
    const FRAMES: usize = 4;

    // Lossless sources, every frame of every one a colour of its own.
    let directory = tempfile::tempdir().unwrap();
    let rate = editor_core::FrameRate::default();
    let colour = |source: usize, frame: usize| [40 + 31 * source as u8, 47 * frame as u8, 123, 255];
    let mut sources = Vec::new();
    for source_index in 0..6 {
        let raw_path = directory.path().join(format!("stack-{source_index}.rgba"));
        let mut raw = std::fs::File::create(&raw_path).unwrap();
        for frame in 0..FRAMES {
            raw.write_all(&colour(source_index, frame).repeat((SIZE[0] * SIZE[1]) as usize))
                .unwrap();
        }
        drop(raw);
        let path = directory.path().join(format!("stack-{source_index}.nut"));
        let status = crate::process::tool_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-f", "rawvideo", "-pixel_format", "rgba"])
            .args(["-video_size", "64x32", "-framerate", "30", "-i"])
            .arg(&raw_path)
            .args(["-an", "-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgra"])
            .args(["-threads", "1", "-filter_threads", "1", "-f", "nut"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .expect("start configured FFmpeg for owned lossless fixture");
        assert!(status.success(), "lossless fixture encoding failed");
        let mut source = Asset::new(path, MediaKind::Video, rate.frame_time(FRAMES as i64));
        source.width = SIZE[0];
        source.height = SIZE[1];
        source.frame_rate = Some(rate);
        source.file_size = std::fs::metadata(&source.path).unwrap().len();
        sources.push(source);
    }

    let service = MediaService::new(|| {}).unwrap();
    // In paint order: out, in, out, in, out, in. The viewer hints the first
    // three it paints.
    let consumers: [PreviewConsumer; 6] = std::array::from_fn(|_| PreviewConsumer::new());
    service
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])])
        .unwrap();

    // The frame the service last took a request at for each consumer, the
    // one it took before that, and the frame each shows.
    let mut admitted: [Option<usize>; 6] = [None; 6];
    let mut before: [Option<usize>; 6] = [None; 6];
    let mut shown: [Option<usize>; 6] = [None; 6];
    let (mut refused, mut turns) = (0u32, 0usize);
    let deadline = Instant::now() + Duration::from_secs(60);
    while shown.iter().any(|frame| *frame != Some(FRAMES - 1)) {
        assert!(
            Instant::now() < deadline,
            "six consumers were not all served: they show {shown:?} of {FRAMES} frames \
             after {turns} turns and {refused} refusals"
        );
        // The clock moves on a frame a turn, whether or not a picture came,
        // and stops on the last frame.
        let clock = turns.min(FRAMES - 1);
        turns += 1;
        for (index, consumer) in consumers.iter().copied().enumerate() {
            if admitted[index] == Some(clock) {
                continue;
            }
            let time = rate.frame_time(clock as i64);
            match service.request_playback_preview(consumer, &sources[index], time, SIZE) {
                Ok(_) => {
                    before[index] = admitted[index];
                    admitted[index] = Some(clock);
                }
                // Its last frame is not decoded yet: it asks again next turn.
                Err(MediaError::Busy) => refused += 1,
                Err(error) => panic!("consumer {index} was turned away for good: {error:?}"),
            }
        }
        while let Some(event) = service.poll() {
            let MediaEvent::FrameReady {
                consumer,
                time,
                frame,
                ..
            } = event
            else {
                panic!("expected a picture, received {event:?}");
            };
            let index = (consumers.iter())
                .position(|known| Some(*known) == consumer)
                .expect("a picture is published for one of the six");
            let frame_index = rate.frame_at(time) as usize;
            assert!(
                [admitted[index], before[index]].contains(&Some(frame_index)),
                "consumer {index} was published frame {frame_index}, which it did not ask for"
            );
            assert_eq!(
                frame.rgba.as_ref(),
                colour(index, frame_index).repeat((SIZE[0] * SIZE[1]) as usize),
                "consumer {index} was published a picture that is not frame {frame_index} of \
                 its own"
            );
            assert!(
                shown[index].is_none_or(|last| last < frame_index),
                "consumer {index} was published frame {frame_index} after {:?}",
                shown[index]
            );
            shown[index] = Some(frame_index);
        }
        thread::sleep(Duration::from_millis(1));
    }
    println!(
        "MEASURE: six playing video consumers over {FRAMES} frames of the clock: {turns} \
         turns, {refused} requests refused as busy"
    );
    assert!(
        refused > 0,
        "a consumer asking again before its frame was decoded is told to wait"
    );
}
