//! Device-free playback admission through real request, dispatch and publication.
use super::*;
use crate::Frame;

fn fixture() -> MediaService {
    MediaService {
        shared: Arc::new(Shared::default()),
        processes: Processes::default(),
    }
}

fn source() -> Asset {
    let mut asset = Asset::new(
        PathBuf::from("owned-playback.png"),
        MediaKind::Image,
        Time(1_000_000),
    );
    asset.width = 1;
    asset.height = 1;
    asset
}

#[derive(Debug, PartialEq)]
struct PendingState {
    lane: Lane,
    generation: u64,
    sequence: u64,
    asset: Asset,
    time: Time,
    size: [u32; 2],
    interactive: bool,
}

#[derive(Debug, PartialEq)]
struct AdmissionState {
    generation: u64,
    sequence: u64,
    latest: BTreeMap<Lane, u64>,
    pending_order: VecDeque<Lane>,
    pending: Vec<PendingState>,
    in_flight: Option<(u64, u64, Lane)>,
    bypasses: u8,
    retention: [Option<PreviewConsumer>; 3],
    maintenance: u64,
    mailbox: (usize, usize),
}

fn admission_state(service: &MediaService) -> AdmissionState {
    let queue = service.shared.queue.lock().unwrap();
    AdmissionState {
        generation: queue.generation,
        sequence: queue.sequence,
        latest: queue.latest.clone(),
        pending_order: queue.pending_order.clone(),
        pending: queue
            .frames
            .iter()
            .map(|(&lane, pending)| PendingState {
                lane,
                generation: pending.job.generation,
                sequence: pending.job.sequence,
                asset: pending.job.asset.clone(),
                time: pending.job.time,
                size: pending.job.size,
                interactive: pending.purpose != FramePurpose::Poster,
            })
            .collect(),
        in_flight: queue
            .in_flight
            .map(|identity| (identity.generation, identity.sequence, identity.lane)),
        bypasses: queue.interactive_bypasses,
        retention: queue.preview_retention,
        maintenance: queue.maintenance,
        mailbox: (queue.events.len(), queue.event_bytes),
    }
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

fn expect_picture(service: &MediaService, job: &FrameJob, color: u8) {
    let MediaEvent::FrameReady {
        generation,
        asset_id,
        time,
        size,
        consumer,
        frame,
    } = service.poll().expect("admitted picture is published")
    else {
        panic!("expected owned playback picture");
    };
    assert_eq!(generation, job.generation);
    assert_eq!(asset_id, job.asset.id);
    assert_eq!(time, job.time);
    assert_eq!(size, job.size);
    assert_eq!(
        consumer,
        match job.lane {
            Lane::Consumer(owner) => Some(owner),
            Lane::Legacy { .. } => None,
        }
    );
    assert_eq!([frame.width, frame.height], [1, 1]);
    assert_eq!(&*frame.rgba, &[color, 0, 0, 255]);
}

#[test]
fn playback_renewals_keep_inflight_job_current_and_publish_it() {
    let service = fixture();
    let asset = source();
    let owner = PreviewConsumer::new();
    service
        .request_playback_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let captured = take(&service);
    let other = PreviewConsumer::new();
    service
        .request_playback_preview(other, &asset, Time(20), [1, 1])
        .unwrap();
    let admitted = admission_state(&service);
    for next in 11..75 {
        assert!(matches!(
            service.request_playback_preview(owner, &asset, Time(next), [1, 1]),
            Err(MediaError::Busy)
        ));
        assert_eq!(admission_state(&service), admitted);
        assert!(
            service
                .shared
                .queue
                .lock()
                .unwrap()
                .frame_current(&captured)
        );
    }
    assert!(publish(&service, &captured, 7));
    expect_picture(&service, &captured, 7);
    assert!(service.shared.queue.lock().unwrap().in_flight.is_none());
    service
        .request_playback_preview(owner, &asset, Time(80), [1, 1])
        .unwrap();
    let first = take(&service);
    assert_eq!(first.lane, Lane::Consumer(other));
    assert!(publish(&service, &first, 8));
    expect_picture(&service, &first, 8);
    let next = take(&service);
    assert_eq!((next.lane, next.time), (Lane::Consumer(owner), Time(80)));
    assert!(publish(&service, &next, 9));
    expect_picture(&service, &next, 9);
    assert!(service.poll().is_none());
}

#[test]
fn pending_playback_refusal_preserves_original_source_bounds_and_admission_order() {
    let service = fixture();
    let asset = source();
    let replacement = source();
    let first = PreviewConsumer::new();
    let second = PreviewConsumer::new();
    service
        .request_playback_preview(first, &asset, Time(10), [1, 1])
        .unwrap();
    service
        .request_playback_preview(second, &asset, Time(20), [1, 1])
        .unwrap();
    let admitted = admission_state(&service);
    assert!(matches!(
        service.request_playback_preview(first, &replacement, Time(30), [2, 3]),
        Err(MediaError::Busy)
    ));
    assert_eq!(admission_state(&service), admitted);
    for (owner, time) in [(first, Time(10)), (second, Time(20))] {
        let job = take(&service);
        assert_eq!((job.lane, job.time), (Lane::Consumer(owner), time));
        assert_eq!(job.asset, asset);
        assert_eq!(job.size, [1, 1]);
        assert!(publish(&service, &job, 1));
        expect_picture(&service, &job, 1);
    }
}

#[test]
fn immediate_seek_source_and_bounds_changes_supersede_inflight_playback() {
    let original = source();
    let different = source();
    for (asset, time, size) in [
        (original.clone(), Time(30), [1, 1]),
        (different, Time(10), [1, 1]),
        (original.clone(), Time(10), [2, 3]),
    ] {
        let service = fixture();
        let owner = PreviewConsumer::new();
        service
            .request_playback_preview(owner, &original, Time(10), [1, 1])
            .unwrap();
        let old = take(&service);
        service.request_preview(owner, &asset, time, size).unwrap();
        assert!(!service.shared.queue.lock().unwrap().frame_current(&old));
        assert!(!publish(&service, &old, 1));
        assert!(service.poll().is_none());
        let replacement = take(&service);
        assert_eq!(replacement.asset, asset);
        assert_eq!((replacement.time, replacement.size), (time, size));
        assert!(publish(&service, &replacement, 2));
        expect_picture(&service, &replacement, 2);
    }
}

#[test]
fn playback_waits_for_existing_seek_demand_without_replacing_it() {
    let service = fixture();
    let asset = source();
    let owner = PreviewConsumer::new();
    service
        .request_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let pending = admission_state(&service);
    assert!(matches!(
        service.request_playback_preview(owner, &asset, Time(20), [1, 1]),
        Err(MediaError::Busy)
    ));
    assert_eq!(admission_state(&service), pending);
    let captured = take(&service);
    let in_flight = admission_state(&service);
    assert!(matches!(
        service.request_playback_preview(owner, &asset, Time(30), [1, 1]),
        Err(MediaError::Busy)
    ));
    assert_eq!(admission_state(&service), in_flight);
    assert!(publish(&service, &captured, 1));
    expect_picture(&service, &captured, 1);
}

#[test]
fn playback_capacity_refusal_is_bounded_and_immediate_replacement_keeps_rank() {
    let service = fixture();
    let asset = source();
    let owners: Vec<_> = (0..32).map(|_| PreviewConsumer::new()).collect();
    service.shared.queue.lock().unwrap().sequence = u64::MAX - 1;
    for &owner in &owners {
        service
            .request_playback_preview(owner, &asset, Time(10), [1, 1])
            .unwrap();
    }
    let full = admission_state(&service);
    let extra = PreviewConsumer::new();
    for owner in [owners[0], extra] {
        assert!(matches!(
            service.request_playback_preview(owner, &asset, Time(20), [2, 3]),
            Err(MediaError::Busy)
        ));
        assert_eq!(admission_state(&service), full);
    }
    service
        .request_preview(owners[0], &asset, Time(30), [2, 3])
        .unwrap();
    let first = take(&service);
    assert_eq!(first.lane, Lane::Consumer(owners[0]));
    assert_eq!((first.time, first.size), (Time(30), [2, 3]));
    assert!(publish(&service, &first, 1));
    expect_picture(&service, &first, 1);
    service
        .request_playback_preview(extra, &asset, Time(40), [1, 1])
        .unwrap();
    let queue = service.shared.queue.lock().unwrap();
    assert_eq!(queue.frames.len(), 32);
    assert_eq!(queue.pending_order.len(), 32);
    assert_eq!(queue.pending_order.back(), Some(&Lane::Consumer(extra)));
}

#[test]
fn playback_has_interactive_priority_without_exceeding_two_oldest_bypasses() {
    let service = fixture();
    let poster = source();
    let asset = source();
    let playback = PreviewConsumer::new();
    let seek = PreviewConsumer::new();
    service.request_poster(&poster, Time::ZERO, [1, 1]).unwrap();
    service
        .request_playback_preview(playback, &asset, Time(10), [1, 1])
        .unwrap();
    service
        .request_preview(seek, &asset, Time(20), [1, 1])
        .unwrap();
    let first = take(&service);
    assert_eq!(first.lane, Lane::Consumer(playback));
    assert!(publish(&service, &first, 1));
    expect_picture(&service, &first, 1);
    service
        .request_playback_preview(playback, &asset, Time(30), [1, 1])
        .unwrap();
    let second = take(&service);
    assert_eq!(second.lane, Lane::Consumer(seek));
    assert!(publish(&service, &second, 2));
    expect_picture(&service, &second, 2);
    service
        .request_playback_preview(PreviewConsumer::new(), &asset, Time(40), [1, 1])
        .unwrap();
    let oldest = take(&service);
    assert_eq!(oldest.asset.id, poster.id);
    assert_eq!(oldest.time, Time::ZERO);
    assert!(publish(&service, &oldest, 3));
    expect_picture(&service, &oldest, 3);
}

#[test]
fn cancellation_releases_playback_admission_and_old_finish_cannot_clear_new_work() {
    let service = fixture();
    let asset = source();
    let owner = PreviewConsumer::new();
    service
        .request_playback_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let old = take(&service);
    let generation = service.cancel();
    assert_ne!(generation, old.generation);
    assert_eq!(
        service
            .request_playback_preview(owner, &asset, Time(20), [1, 1])
            .unwrap(),
        generation
    );
    let next = take(&service);
    assert!(!publish(&service, &old, 1));
    assert!(service.shared.queue.lock().unwrap().in_flight == Some(next.identity()));
    assert!(publish(&service, &next, 2));
    expect_picture(&service, &next, 2);
    assert!(service.poll().is_none());
}

#[test]
fn retirement_invalidates_playback_and_releases_admission_after_its_old_job_finishes() {
    let service = fixture();
    let asset = source();
    let owner = PreviewConsumer::new();
    let other = PreviewConsumer::new();
    service
        .set_preview_retention([Some(owner), None, None])
        .unwrap();
    service
        .request_playback_preview(owner, &asset, Time(1), [1, 1])
        .unwrap();
    assert!(publish(&service, &take(&service), 1));
    service
        .request_playback_preview(owner, &asset, Time(10), [1, 1])
        .unwrap();
    let retired = take(&service);
    service
        .request_playback_preview(other, &asset, Time(20), [1, 1])
        .unwrap();
    service.retire_preview(owner);
    assert!(
        service.poll().is_none(),
        "retired owner's old picture is removed"
    );
    let state = admission_state(&service);
    assert!(!state.latest.contains_key(&Lane::Consumer(owner)));
    assert_eq!(state.retention, [None; 3]);
    assert!(matches!(
        service.request_playback_preview(owner, &asset, Time(30), [1, 1]),
        Err(MediaError::Busy)
    ));
    assert_eq!(admission_state(&service), state);
    assert!(!publish(&service, &retired, 2));
    service
        .request_playback_preview(owner, &asset, Time(40), [1, 1])
        .unwrap();
    for (expected, time) in [(other, Time(20)), (owner, Time(40))] {
        let job = take(&service);
        assert_eq!((job.lane, job.time), (Lane::Consumer(expected), time));
        assert!(publish(&service, &job, 3));
        expect_picture(&service, &job, 3);
    }
}
