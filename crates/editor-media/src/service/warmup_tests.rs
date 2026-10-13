//! Cold preparation crosses the actual queue and decoder-manager boundaries.
use super::*;
use crate::decode::warmup_tests::Videos;

fn service(processes: Processes) -> MediaService {
    MediaService {
        shared: Arc::new(Shared::default()),
        processes,
    }
}

#[test]
fn warmup_snapshot_keeps_admission_order_skips_posters_and_owns_four_pending_jobs_at_most() {
    let videos = Videos::new(1);
    let service = service(videos.processes.clone());
    let consumers: [PreviewConsumer; 7] = std::array::from_fn(|_| PreviewConsumer::new());
    service
        .request_preview(consumers[0], &videos.assets[0], Time::ZERO, [64, 32])
        .unwrap();
    service
        .request_poster(&videos.assets[0], Time::ZERO, [64, 32])
        .unwrap();
    service
        .request_preview(
            PreviewConsumer::new(),
            &videos.image(),
            Time::ZERO,
            [64, 32],
        )
        .unwrap();
    for consumer in consumers[1..].iter().copied() {
        service
            .request_playback_preview(consumer, &videos.assets[0], Time::ZERO, [64, 32])
            .unwrap();
    }
    let mut queue = service.shared.queue.lock().unwrap();
    let job = queue.take_pending_frame().unwrap().job;
    let original_order = queue.pending_order.clone();
    let original_sequence = queue.sequence;
    let original_inflight = queue.in_flight;
    let jobs = queue.video_warmup_jobs(&job);
    assert_eq!(jobs.len(), 4);
    for (prepared, consumer) in jobs.iter().zip(consumers[1..5].iter()) {
        assert_eq!(prepared.lane, Lane::Consumer(*consumer));
        assert!(queue.frame_current(prepared));
    }
    assert_eq!(queue.pending_order, original_order);
    assert_eq!(queue.sequence, original_sequence);
    assert!(queue.in_flight == original_inflight);
}

#[test]
fn worker_cohort_opens_five_pipes_before_reading_and_delivers_exact_requested_frames() {
    let mut videos = Videos::new(5);
    let service = service(videos.processes.clone());
    let consumers: [PreviewConsumer; 5] = std::array::from_fn(|_| PreviewConsumer::new());
    for (source, consumer) in consumers.iter().copied().enumerate() {
        service
            .request_playback_preview(consumer, &videos.assets[source], Time::ZERO, [64, 32])
            .unwrap();
    }
    let PendingFrame { job, purpose } = service
        .shared
        .queue
        .lock()
        .unwrap()
        .take_pending_frame()
        .unwrap();
    prepare_video_cohort(&service.shared, &mut videos.pool, &job, purpose, 0).unwrap();
    assert_eq!(videos.processes.started_tools(), 5);
    let mut next = Some(job);
    for (source, consumer) in consumers.iter().copied().enumerate() {
        let job = next.take().unwrap_or_else(|| {
            service
                .shared
                .queue
                .lock()
                .unwrap()
                .take_pending_frame()
                .unwrap()
                .job
        });
        let frame = videos
            .pool
            .frame(&job, || {
                service.shared.queue.lock().unwrap().frame_current(&job)
            })
            .unwrap();
        videos.assert_pixels(&frame, source, 0);
        assert!(publish_result(&service.shared, &job, Ok(frame)).unwrap());
        let MediaEvent::FrameReady {
            consumer: returned_consumer,
            time,
            asset_id,
            frame,
            ..
        } = service.poll().unwrap()
        else {
            panic!("current warmup frame was not published");
        };
        assert_eq!(
            (returned_consumer, time, asset_id),
            (Some(consumer), Time::ZERO, videos.assets[source].id)
        );
        videos.assert_pixels(&frame, source, 0);
        assert_eq!(videos.processes.started_tools(), 5);
    }
}

#[test]
fn stale_seek_resize_retirement_and_generation_stop_preparation() {
    for change in 0..4 {
        let mut videos = Videos::new(1);
        let service = service(videos.processes.clone());
        let consumer = PreviewConsumer::new();
        service
            .request_playback_preview(consumer, &videos.assets[0], Time::ZERO, [64, 32])
            .unwrap();
        let PendingFrame { job, purpose } = service
            .shared
            .queue
            .lock()
            .unwrap()
            .take_pending_frame()
            .unwrap();
        match change {
            0 => {
                service
                    .request_preview(consumer, &videos.assets[0], Time(33_333), [64, 32])
                    .unwrap();
            }
            1 => {
                service
                    .request_preview(consumer, &videos.assets[0], Time::ZERO, [32, 16])
                    .unwrap();
            }
            2 => service.retire_preview(consumer),
            _ => {
                service.cancel();
            }
        }
        prepare_video_cohort(&service.shared, &mut videos.pool, &job, purpose, 0).unwrap();
        assert_eq!(videos.processes.started_tools(), 0);
    }
}

#[test]
fn poster_dispatch_does_not_prepare_other_owners() {
    let mut videos = Videos::new(1);
    let service = service(videos.processes.clone());
    service
        .request_poster(&videos.assets[0], Time::ZERO, [64, 32])
        .unwrap();
    let PendingFrame { job, purpose } = service
        .shared
        .queue
        .lock()
        .unwrap()
        .take_pending_frame()
        .unwrap();
    service
        .request_playback_preview(
            PreviewConsumer::new(),
            &videos.assets[0],
            Time::ZERO,
            [64, 32],
        )
        .unwrap();
    prepare_video_cohort(&service.shared, &mut videos.pool, &job, purpose, 0).unwrap();
    assert_eq!(videos.processes.started_tools(), 0);
    let frame = videos.pool.frame(&job, || true).unwrap();
    videos.assert_pixels(&frame, 0, 0);
    assert_eq!(videos.processes.started_tools(), 1);
}

#[test]
fn unrelated_maintenance_must_not_discard_current_paused_picture() {
    let mut videos = Videos::new(1);
    let service = service(videos.processes.clone());
    let consumer = PreviewConsumer::new();
    let other = PreviewConsumer::new();
    service
        .request_preview(consumer, &videos.assets[0], Time::ZERO, [64, 32])
        .unwrap();
    service
        .request_preview(other, &videos.assets[0], Time::ZERO, [64, 32])
        .unwrap();
    let PendingFrame { job, purpose } = service
        .shared
        .queue
        .lock()
        .unwrap()
        .take_pending_frame()
        .unwrap();
    service.retire_preview(other);
    prepare_video_cohort(&service.shared, &mut videos.pool, &job, purpose, 0).unwrap();
    assert_eq!(videos.processes.started_tools(), 0);
    let frame = videos
        .pool
        .frame(&job, || {
            service.shared.queue.lock().unwrap().frame_current(&job)
        })
        .unwrap();
    videos.assert_pixels(&frame, 0, 0);
    assert!(publish_result(&service.shared, &job, Ok(frame)).unwrap());
    let MediaEvent::FrameReady {
        consumer: returned, ..
    } = service.poll().unwrap()
    else {
        panic!("maintenance discarded the still-current finite picture");
    };
    assert_eq!(returned, Some(consumer));
}

#[test]
fn captured_cohort_stops_on_any_new_request_retirement_hint_or_generation() {
    for change in 0..6 {
        let videos = Videos::new(1);
        let service = service(videos.processes.clone());
        let current = PreviewConsumer::new();
        let pending = PreviewConsumer::new();
        service
            .request_preview(current, &videos.assets[0], Time::ZERO, [64, 32])
            .unwrap();
        service
            .request_playback_preview(pending, &videos.assets[0], Time::ZERO, [64, 32])
            .unwrap();
        let (job, jobs, sequence, maintenance) = {
            let mut queue = service.shared.queue.lock().unwrap();
            let job = queue.take_pending_frame().unwrap().job;
            let jobs = queue.video_warmup_jobs(&job);
            (job, jobs, queue.sequence, queue.maintenance)
        };
        assert!(video_warmup_current(
            &service.shared,
            &job,
            &jobs,
            sequence,
            maintenance
        ));
        match change {
            0 => {
                service
                    .request_preview(current, &videos.assets[0], Time(33_333), [64, 32])
                    .unwrap();
            }
            1 => {
                service
                    .request_preview(pending, &videos.assets[0], Time::ZERO, [32, 16])
                    .unwrap();
            }
            2 => {
                service
                    .request_preview(
                        PreviewConsumer::new(),
                        &videos.assets[0],
                        Time::ZERO,
                        [64, 32],
                    )
                    .unwrap();
            }
            3 => service.retire_preview(pending),
            4 => service
                .set_preview_retention([Some(current), None, None])
                .unwrap(),
            _ => {
                service.cancel();
            }
        }
        assert!(!video_warmup_current(
            &service.shared,
            &job,
            &jobs,
            sequence,
            maintenance
        ));
        assert_eq!(videos.processes.started_tools(), 0);
    }
}
