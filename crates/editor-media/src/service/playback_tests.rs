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

/// A delayed first decode leaves a playing clock well ahead of its pipe. It
/// must catch up through that pipe rather than paying cold startup each turn.
#[test]
fn delayed_playback_catches_up_without_restarting_while_an_explicit_seek_still_restarts() {
    use std::{io::Write, process::Child};
    const SIZE: [u32; 2] = [64, 32];
    const FRAMES: i64 = 300;

    struct Encoder(Child);
    impl Drop for Encoder {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let raw = directory.path().join("delayed-playback.rgba");
    let pixels = |index: i64| {
        [index as u8, (index >> 8) as u8, 131, 211].repeat((SIZE[0] * SIZE[1]) as usize)
    };
    let mut input = std::fs::File::create(&raw).unwrap();
    for index in 0..FRAMES {
        input.write_all(&pixels(index)).unwrap();
    }
    drop(input);
    let path = directory.path().join("delayed-playback.nut");
    let mut encoder = Encoder(
        crate::process::tool_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-f", "rawvideo", "-pixel_format", "rgba"])
            .args(["-video_size", "64x32", "-framerate", "60", "-i"])
            .arg(&raw)
            .args(["-an", "-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgra"])
            .args(["-threads", "1", "-filter_threads", "1", "-f", "nut"])
            .arg(&path)
            .spawn()
            .expect("owned playback catch-up encoder"),
    );
    assert!(encoder.0.wait().unwrap().success());
    drop(encoder);
    let rate = editor_core::FrameRate {
        numerator: 60,
        denominator: 1,
    };
    let mut asset = Asset::new(path, MediaKind::Video, rate.frame_time(FRAMES));
    asset.width = SIZE[0];
    asset.height = SIZE[1];
    asset.frame_rate = Some(rate);
    asset.file_size = std::fs::metadata(&asset.path).unwrap().len();
    let service = MediaService::new(|| {}).unwrap();
    let consumer = PreviewConsumer::new();
    service
        .set_preview_retention([Some(consumer), None, None])
        .unwrap();
    let picture = |index: i64| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = service.poll() {
                let MediaEvent::FrameReady {
                    asset_id,
                    time,
                    consumer: returned,
                    frame,
                    ..
                } = event
                else {
                    panic!("expected exact catch-up picture, received {event:?}");
                };
                assert_eq!((asset_id, returned), (asset.id, Some(consumer)));
                assert_eq!(time, rate.frame_time(index));
                assert_eq!([frame.width, frame.height], SIZE);
                assert_eq!(frame.rgba.as_ref(), pixels(index));
                break;
            }
            assert!(Instant::now() < deadline, "catch-up picture timed out");
            thread::sleep(Duration::from_millis(1));
        }
    };
    service
        .request_preview(consumer, &asset, Time::ZERO, SIZE)
        .unwrap();
    picture(0);
    assert_eq!(service.processes.started_tools(), 1);
    // Model a 600 ms clock advance during startup without a timing-sensitive
    // sleep. This crosses the interactive twelve-source-frame seek window.
    service
        .request_playback_preview(consumer, &asset, rate.frame_time(36), SIZE)
        .unwrap();
    picture(36);
    assert_eq!(
        service.processes.started_tools(),
        1,
        "advancing playback must not repeatedly restart its already warm source"
    );
    // Dense sources can stay more than one second behind after a cold seek.
    // A second clock advance must keep catching up through that same pipe.
    service
        .request_playback_preview(consumer, &asset, rate.frame_time(126), SIZE)
        .unwrap();
    picture(126);
    assert_eq!(
        service.processes.started_tools(),
        1,
        "playback must recover when cold decoding leaves the clock 1.5 seconds ahead"
    );
    service
        .request_preview(consumer, &asset, rate.frame_time(162), SIZE)
        .unwrap();
    picture(162);
    assert_eq!(
        service.processes.started_tools(),
        2,
        "an explicit seek still starts at its requested frame"
    );
    // The larger playback window remains finite: more than two source seconds
    // skips directly to its target instead of draining an arbitrary interval.
    service
        .request_playback_preview(consumer, &asset, rate.frame_time(289), SIZE)
        .unwrap();
    picture(289);
    assert_eq!(service.processes.started_tools(), 3);
    service
        .request_playback_preview(consumer, &asset, rate.frame_time(3), SIZE)
        .unwrap();
    picture(3);
    assert_eq!(
        service.processes.started_tools(),
        4,
        "playback intent cannot reuse a forwards-only pipe for an uncached earlier frame"
    );
    let processes = service.processes.clone();
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(5);
    while processes.retained_monitors() != 0 {
        assert!(
            Instant::now() < deadline,
            "owned playback decoder survived close"
        );
        thread::sleep(Duration::from_millis(1));
    }

    // A cohort can contain both a seek and advancing playback. Preparing it
    // must preserve each queued owner's intent, regardless of which one the
    // worker dispatched first. Use the same actual source with independent
    // asset identities so shared pixel-cache hits cannot hide a restarted pipe.
    let service = fixture();
    let mut pool = DecoderPool::new(service.processes.clone());
    let owners = [PreviewConsumer::new(), PreviewConsumer::new()];
    let mut other = asset.clone();
    other.id = uuid::Uuid::new_v4();
    let assets = [&asset, &other];
    let finish = |pending: PendingFrame, pool: &mut DecoderPool| {
        let frame = if pending.purpose == FramePurpose::Playback {
            pool.playback_frame(&pending.job, || true)
        } else {
            pool.frame(&pending.job, || true)
        }
        .unwrap();
        let index = rate.frame_at(pending.job.time);
        assert_eq!(frame.rgba.as_ref(), pixels(index));
        assert!(publish_result(&service.shared, &pending.job, Ok(frame)).unwrap());
        let MediaEvent::FrameReady {
            asset_id,
            consumer,
            time,
            ..
        } = service.poll().unwrap()
        else {
            panic!("expected owned mixed-cohort picture");
        };
        assert_eq!(asset_id, pending.job.asset.id);
        assert_eq!(time, pending.job.time);
        assert_eq!(consumer.map(Lane::Consumer), Some(pending.job.lane));
    };
    for (&owner, &asset) in owners.iter().zip(&assets) {
        service
            .request_preview(owner, asset, Time::ZERO, SIZE)
            .unwrap();
        let pending = service
            .shared
            .queue
            .lock()
            .unwrap()
            .take_pending_frame()
            .unwrap();
        finish(pending, &mut pool);
    }
    assert_eq!(service.processes.started_tools(), 2);
    for (index, first_playing, expected_starts) in [(36, true, 3), (72, false, 4)] {
        for (owner_index, (&owner, &asset)) in owners.iter().zip(&assets).enumerate() {
            let playing = if owner_index == 0 {
                first_playing
            } else {
                !first_playing
            };
            if playing {
                service.request_playback_preview(owner, asset, rate.frame_time(index), SIZE)
            } else {
                service.request_preview(owner, asset, rate.frame_time(index), SIZE)
            }
            .unwrap();
        }
        let pending = service
            .shared
            .queue
            .lock()
            .unwrap()
            .take_pending_frame()
            .unwrap();
        prepare_video_cohort(&service.shared, &mut pool, &pending.job, pending.purpose, 0).unwrap();
        assert_eq!(
            service.processes.started_tools(),
            expected_starts,
            "warmup must apply the purpose of each pending owner independently"
        );
        finish(pending, &mut pool);
        let pending = service
            .shared
            .queue
            .lock()
            .unwrap()
            .take_pending_frame()
            .unwrap();
        finish(pending, &mut pool);
        assert_eq!(service.processes.started_tools(), expected_starts);
    }
    // Superseding a request while it drains the enlarged forward interval
    // stops between frames, and the new explicit seek keeps its own identity.
    service
        .request_playback_preview(owners[0], &asset, rate.frame_time(100), SIZE)
        .unwrap();
    let pending = service
        .shared
        .queue
        .lock()
        .unwrap()
        .take_pending_frame()
        .unwrap();
    let checks = std::cell::Cell::new(0);
    let result = pool.playback_frame(&pending.job, || {
        checks.set(checks.get() + 1);
        if checks.get() == 5 {
            service
                .request_preview(owners[0], &asset, rate.frame_time(110), SIZE)
                .unwrap();
        }
        service
            .shared
            .queue
            .lock()
            .unwrap()
            .frame_current(&pending.job)
    });
    assert!(matches!(result, Err(MediaError::Cancelled)));
    assert_eq!(checks.get(), 5);
    assert!(!publish_result(&service.shared, &pending.job, result).unwrap());
    assert!(service.poll().is_none());
    let seek = service
        .shared
        .queue
        .lock()
        .unwrap()
        .take_pending_frame()
        .unwrap();
    assert_eq!(seek.job.time, rate.frame_time(110));
    assert!(seek.purpose == FramePurpose::Interactive);
    finish(seek, &mut pool);
    assert_eq!(service.processes.started_tools(), 5);
    pool.clear();
}
