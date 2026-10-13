//! Scheduling regressions at request, single-worker dispatch, publication and poll.
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
        PathBuf::from("owned-dispatch.png"),
        MediaKind::Image,
        Time(1_000_000),
    )
}

fn complete(service: &MediaService, expected_time: Time) -> AssetId {
    let job = service.shared.queue.lock().unwrap().take_frame().unwrap();
    assert_eq!(
        job.time, expected_time,
        "latest pending source time is dispatched"
    );
    assert!(service.shared.queue.lock().unwrap().frame_current(&job));
    assert!(
        publish_result(
            &service.shared,
            &job,
            Ok(Frame::new(1, 1, vec![7, 0, 0, 255]).unwrap())
        )
        .unwrap()
    );
    let MediaEvent::FrameReady {
        asset_id,
        time,
        frame,
        ..
    } = service.poll().unwrap()
    else {
        panic!("expected dispatched frame");
    };
    assert_eq!(time, expected_time);
    assert_eq!(&*frame.rgba, &[7, 0, 0, 255]);
    assert!(service.poll().is_none());
    assert!(service.shared.queue.lock().unwrap().in_flight.is_none());
    assert_eq!(asset_id, job.asset.id);
    asset_id
}

#[test]
fn renewed_interactive_cohort_serves_every_admitted_lane_within_three_opportunities() {
    let service = fixture();
    let assets = [asset(), asset(), asset()];
    let mut served = Vec::new();
    for turn in 0..3 {
        for asset in &assets {
            service
                .request_frame(asset, Time(turn * 100), [1, 1])
                .unwrap();
        }
        served.push(complete(&service, Time(turn * 100)));
    }
    assert_eq!(
        served,
        assets.iter().map(|asset| asset.id).collect::<Vec<_>>(),
        "renewing later lanes must not starve an older accepted lane"
    );
}

#[test]
fn pending_replacement_keeps_its_admission_position_with_latest_source_time() {
    let service = fixture();
    let first = asset();
    let second = asset();
    service.request_frame(&first, Time::ZERO, [1, 1]).unwrap();
    service.request_frame(&second, Time::ZERO, [1, 1]).unwrap();
    service.request_frame(&first, Time(20), [1, 1]).unwrap();
    service.request_frame(&second, Time(30), [1, 1]).unwrap();
    assert_eq!(complete(&service, Time(20)), first.id);
    assert_eq!(complete(&service, Time(30)), second.id);
}

#[test]
fn admission_order_survives_wrapping_request_sequence() {
    let service = fixture();
    service.shared.queue.lock().unwrap().sequence = u64::MAX - 1;
    let first = asset();
    let second = asset();
    service.request_frame(&first, Time(10), [1, 1]).unwrap();
    service.request_frame(&second, Time(20), [1, 1]).unwrap();
    service.request_frame(&first, Time(30), [1, 1]).unwrap();
    assert_eq!(complete(&service, Time(30)), first.id);
    assert_eq!(complete(&service, Time(20)), second.id);
}

#[test]
fn explicit_interactive_demand_bypasses_older_and_newer_posters_at_equal_size() {
    let service = fixture();
    let older = asset();
    let interactive = asset();
    let newer = asset();
    service.request_poster(&older, Time(1), [1, 1]).unwrap();
    service
        .request_frame(&interactive, Time(2), [1, 1])
        .unwrap();
    service.request_poster(&newer, Time(3), [1, 1]).unwrap();
    assert_eq!(
        complete(&service, Time(2)),
        interactive.id,
        "declared purpose, not requested size or newest sequence, selects priority"
    );
}

#[test]
fn global_oldest_is_selected_after_two_interactive_bypasses() {
    let service = fixture();
    let poster = asset();
    service.request_poster(&poster, Time::ZERO, [1, 1]).unwrap();
    for turn in 0..2 {
        let interactive = asset();
        let newer_poster = asset();
        service
            .request_frame(&interactive, Time(turn + 1), [1, 1])
            .unwrap();
        service
            .request_poster(&newer_poster, Time(100 + turn), [1, 1])
            .unwrap();
        assert_eq!(complete(&service, Time(turn + 1)), interactive.id);
        // Replacing P cannot erase its age or the bypass debt.
        service
            .request_poster(&poster, Time(turn + 10), [1, 1])
            .unwrap();
    }
    let newest = asset();
    service.request_frame(&newest, Time(200), [1, 1]).unwrap();
    assert_eq!(complete(&service, Time(11)), poster.id);
}

#[test]
fn changing_pending_purpose_preserves_global_admission_rank_and_bypass_debt() {
    let service = fixture();
    let first = asset();
    service.request_poster(&first, Time::ZERO, [1, 1]).unwrap();
    for turn in 0..2 {
        let interactive = asset();
        service
            .request_frame(&interactive, Time(turn + 1), [1, 1])
            .unwrap();
        assert_eq!(complete(&service, Time(turn + 1)), interactive.id);
    }
    service.request_frame(&first, Time(99), [1, 1]).unwrap();
    let newer = asset();
    service.request_frame(&newer, Time(100), [1, 1]).unwrap();
    assert_eq!(complete(&service, Time(99)), first.id);
}

#[test]
fn full_poster_queue_still_rejects_new_lanes_and_accepts_pending_replacement() {
    let service = fixture();
    let assets: Vec<_> = (0..32).map(|_| asset()).collect();
    for asset in &assets {
        service.request_poster(asset, Time::ZERO, [1, 1]).unwrap();
    }
    assert!(matches!(
        service.request_frame(&asset(), Time::ZERO, [1, 1]),
        Err(MediaError::Busy)
    ));
    service
        .request_poster(&assets[0], Time(100), [1, 1])
        .unwrap();
    assert_eq!(service.shared.queue.lock().unwrap().frames.len(), 32);
    for (index, asset) in assets.iter().enumerate() {
        assert_eq!(
            complete(&service, if index == 0 { Time(100) } else { Time::ZERO }),
            asset.id
        );
    }
}

#[test]
fn poster_and_interactive_methods_coalesce_same_lane_and_latest_purpose() {
    let service = fixture();
    let same = asset();
    let other = asset();
    service.request_poster(&same, Time(1), [1, 1]).unwrap();
    service.request_poster(&other, Time(2), [1, 1]).unwrap();
    service.request_frame(&same, Time(3), [1, 1]).unwrap();
    assert_eq!(service.shared.queue.lock().unwrap().frames.len(), 2);
    assert_eq!(complete(&service, Time(3)), same.id);
    assert_eq!(complete(&service, Time(2)), other.id);
}

#[test]
fn a_full_mixed_cohort_with_renewed_interactive_work_respects_the_rank_bound() {
    let service = fixture();
    let older: Vec<_> = (0..29).map(|_| asset()).collect();
    for asset in &older {
        service.request_poster(asset, Time::ZERO, [1, 1]).unwrap();
    }
    let target = asset();
    service.request_poster(&target, Time::ZERO, [1, 1]).unwrap();
    let interactive = [asset(), asset()];
    let rank_bound = 3 * (older.len() + 1);
    let mut served_target = false;
    for opportunity in 1..=rank_bound {
        let time = Time(opportunity as i64);
        service.request_poster(&target, time, [1, 1]).unwrap();
        for asset in &interactive {
            service.request_frame(asset, time, [1, 1]).unwrap();
        }
        assert!(service.shared.queue.lock().unwrap().frames.len() <= 32);
        // Older unrenewed poster jobs still request Time::ZERO.
        let job = service.shared.queue.lock().unwrap().take_frame().unwrap();
        if job.asset.id == target.id {
            assert_eq!(job.time, time);
            served_target = true;
        }
        assert!(
            publish_result(
                &service.shared,
                &job,
                Ok(Frame::new(1, 1, vec![0; 4]).unwrap())
            )
            .unwrap()
        );
        assert!(matches!(
            service.poll(),
            Some(MediaEvent::FrameReady { .. })
        ));
        if served_target {
            break;
        }
    }
    assert!(
        served_target,
        "accepted continuously pending episode exceeds its dispatch rank bound"
    );
}

#[test]
fn failed_publication_does_not_reset_bypass_debt_and_cancel_starts_a_fresh_cohort() {
    let service = fixture();
    let poster = asset();
    service.request_poster(&poster, Time::ZERO, [1, 1]).unwrap();
    for _ in 0..2 {
        service.request_frame(&asset(), Time::ZERO, [1, 1]).unwrap();
        let job = service.shared.queue.lock().unwrap().take_frame().unwrap();
        assert_ne!(job.asset.id, poster.id);
        assert!(!publish_result(&service.shared, &job, Err(MediaError::Cancelled)).unwrap());
        assert!(service.poll().is_none());
    }
    service.request_frame(&asset(), Time::ZERO, [1, 1]).unwrap();
    assert_eq!(complete(&service, Time::ZERO), poster.id);
    let before = service.generation();
    assert_ne!(service.cancel(), before);
    let older = asset();
    let newer = asset();
    service.request_poster(&older, Time::ZERO, [1, 1]).unwrap();
    service.request_frame(&newer, Time::ZERO, [1, 1]).unwrap();
    assert_eq!(complete(&service, Time::ZERO), newer.id);
    assert_eq!(complete(&service, Time::ZERO), older.id);
    assert!(service.shared.queue.lock().unwrap().take_frame().is_none());
}
