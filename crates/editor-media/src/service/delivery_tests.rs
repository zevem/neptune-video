//! Device-free regressions at the real request, dispatch and publication seams.

use super::*;
use crate::Frame;
use std::{
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);
const LARGE_SIZE: [u32; 2] = [1920, 1920];

#[derive(Debug)]
enum PublicationProgress {
    Waiting,
    Finished(Result<bool, MediaError>),
}

/// No import/decoder workers or native resources are created. The optional
/// thread owns only one call to the production publication seam.
struct DeliveryFixture {
    service: Option<MediaService>,
    shared: Arc<Shared>,
    publication: Option<JoinHandle<()>>,
    progress: Option<Receiver<PublicationProgress>>,
}

impl DeliveryFixture {
    fn new() -> Self {
        let shared = Arc::new(Shared::default());
        Self {
            service: Some(MediaService {
                shared: Arc::clone(&shared),
                processes: Processes::default(),
            }),
            shared,
            publication: None,
            progress: None,
        }
    }

    fn service(&self) -> &MediaService {
        self.service.as_ref().expect("owned service is open")
    }

    fn take_job(&self) -> FrameJob {
        self.shared
            .queue
            .lock()
            .expect("owned queue")
            .take_frame()
            .expect("an accepted frame request remains dispatchable")
    }

    fn request_job(&self, asset: &Asset, time: Time, size: [u32; 2]) -> FrameJob {
        self.service()
            .request_frame(asset, time, size)
            .expect("accepted request");
        let job = self.take_job();
        assert_eq!(
            job.asset.id, asset.id,
            "this helper requires no competing pending lane"
        );
        job
    }

    fn current(&self, job: &FrameJob) -> bool {
        self.shared
            .queue
            .lock()
            .expect("owned queue")
            .frame_current(job)
    }

    fn frame_ids(&self) -> Vec<AssetId> {
        self.shared
            .queue
            .lock()
            .expect("owned queue")
            .events
            .iter()
            .filter_map(|event| match event {
                MediaEvent::FrameReady { asset_id, .. } => Some(*asset_id),
                _ => None,
            })
            .collect()
    }

    fn mailbox_usage(&self) -> (usize, usize) {
        let queue = self.shared.queue.lock().expect("owned queue");
        (queue.events.len(), queue.event_bytes)
    }

    fn start_publication(&mut self, job: FrameJob, frame: Frame) {
        assert!(self.publication.is_none(), "one owned publisher at a time");
        let shared = Arc::clone(&self.shared);
        // Only the tiny Waiting/Finished observations use this unbounded
        // channel. Sending in the hook cannot wait while holding the queue.
        let (sender, receiver) = mpsc::channel();
        self.progress = Some(receiver);
        self.publication = Some(thread::spawn(move || {
            let result = publish_result_before_wait(&shared, &job, Ok(frame), || {
                let _ = sender.send(PublicationProgress::Waiting);
            });
            let _ = sender.send(PublicationProgress::Finished(result));
        }));
    }

    fn expect_wait(&self) {
        let progress = self
            .progress
            .as_ref()
            .expect("owned observer")
            .recv_timeout(OBSERVATION_TIMEOUT)
            .expect("publication made progress");
        assert!(
            matches!(progress, PublicationProgress::Waiting),
            "full mailbox must reach its before-wait seam, got {progress:?}"
        );
        // This observes the before-wait hook, not an OS-level parked thread.
        // Subsequent service operations take the same mutex before notifying.
    }

    fn finish_publication(&mut self) -> Result<bool, MediaError> {
        let deadline = Instant::now() + OBSERVATION_TIMEOUT;
        let result = loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("publication did not finish after a service wake");
            let progress = self
                .progress
                .as_ref()
                .expect("owned observer")
                .recv_timeout(remaining)
                .expect("publication completion observed");
            match progress {
                PublicationProgress::Waiting => continue,
                PublicationProgress::Finished(result) => break result,
            }
        };
        self.publication
            .take()
            .expect("owned publisher")
            .join()
            .expect("publisher did not panic");
        result
    }

    fn close(&mut self) {
        drop(self.service.take());
    }
}

impl Drop for DeliveryFixture {
    fn drop(&mut self) {
        // The actual service Drop sets stopped and notifies before joining our
        // only owned thread. This also runs while an assertion unwinds.
        self.close();
        if let Some(publication) = self.publication.take() {
            let _ = publication.join();
        }
    }
}

fn image_asset() -> Asset {
    let mut asset = Asset::new(
        PathBuf::from("owned-delivery-fixture.png"),
        MediaKind::Image,
        Time(1_000_000),
    );
    asset.width = 1920;
    asset.height = 1920;
    asset
}

fn small_frame(color: u8) -> Frame {
    Frame::new(1, 1, vec![color, 0, 0, 255]).expect("owned RGBA frame")
}

fn expect_frame(event: MediaEvent, asset_id: AssetId, time: Time, size: [u32; 2]) -> Frame {
    let MediaEvent::FrameReady {
        asset_id: actual_id,
        time: actual_time,
        size: actual_size,
        frame,
        ..
    } = event
    else {
        panic!("expected owned frame result");
    };
    assert_eq!(actual_id, asset_id);
    assert_eq!(actual_time, time);
    assert_eq!(actual_size, size);
    frame
}

fn complete_small_job(fixture: &DeliveryFixture, job: &FrameJob) {
    assert!(publish_result(&fixture.shared, job, Ok(small_frame(1))).expect("owned publication"));
    expect_frame(
        fixture.service().poll().expect("published result"),
        job.asset.id,
        job.time,
        job.size,
    );
}

/// Each queued frame owns an Arc clone of one valid 14,745,600-byte payload.
/// Budget accounting remains logical per event, as in production.
fn filled_frame_mailbox() -> (DeliveryFixture, Frame, Vec<Asset>) {
    let fixture = DeliveryFixture::new();
    let bytes = frame_bytes(LARGE_SIZE[0], LARGE_SIZE[1]).expect("bounded dimensions");
    let frame =
        Frame::new(LARGE_SIZE[0], LARGE_SIZE[1], vec![0; bytes]).expect("owned square RGBA frame");
    assert_eq!(frame.byte_len(), 14_745_600);
    assert!(4 * frame.byte_len() <= crate::FRAME_CACHE_BYTES);
    assert!(5 * frame.byte_len() > crate::FRAME_CACHE_BYTES);
    let assets: Vec<_> = (0..5).map(|_| image_asset()).collect();
    for (index, asset) in assets[..4].iter().enumerate() {
        let job = fixture.request_job(asset, Time(index as i64), LARGE_SIZE);
        assert!(
            publish_result(&fixture.shared, &job, Ok(frame.clone()))
                .expect("initial frame fits the mailbox")
        );
    }
    assert_eq!(fixture.mailbox_usage(), (4, 4 * frame.byte_len()));
    (fixture, frame, assets)
}

#[test]
fn playback_renewals_preserve_an_admitted_byte_blocked_publication() {
    let (mut fixture, frame, assets) = filled_frame_mailbox();
    let owner = PreviewConsumer::new();
    fixture
        .service()
        .request_playback_preview(owner, &assets[4], Time(10), LARGE_SIZE)
        .unwrap();
    let job = fixture.take_job();
    let admitted_sequence = job.sequence;
    fixture.start_publication(job.clone(), frame.clone());
    fixture.expect_wait();
    for time in [Time(20), Time(30), Time(40)] {
        assert!(matches!(
            fixture
                .service()
                .request_playback_preview(owner, &assets[4], time, LARGE_SIZE),
            Err(MediaError::Busy)
        ));
        let queue = fixture.shared.queue.lock().unwrap();
        assert_eq!(queue.sequence, admitted_sequence);
        assert_eq!(
            queue.latest.get(&Lane::Consumer(owner)),
            Some(&admitted_sequence)
        );
        assert!(queue.in_flight == Some(job.identity()));
        assert!(queue.frames.is_empty() && queue.pending_order.is_empty());
        assert!(queue.frame_current(&job));
    }
    fixture
        .service()
        .poll()
        .expect("release one owned mailbox slot");
    assert!(fixture.finish_publication().unwrap());
    for asset in &assets[1..4] {
        let event = fixture.service().poll().unwrap();
        assert!(matches!(event, MediaEvent::FrameReady { asset_id, .. } if asset_id == asset.id));
    }
    let MediaEvent::FrameReady {
        generation,
        asset_id,
        time,
        size,
        consumer,
        frame: published,
    } = fixture.service().poll().unwrap()
    else {
        panic!("held admitted playback frame must survive renewal attempts");
    };
    assert_eq!(
        (generation, asset_id, time, size, consumer),
        (
            job.generation,
            assets[4].id,
            Time(10),
            LARGE_SIZE,
            Some(owner)
        )
    );
    assert_eq!(&*published.rgba, &*frame.rgba);
    assert!(fixture.service().poll().is_none());
    fixture
        .service()
        .request_playback_preview(owner, &assets[4], Time(50), LARGE_SIZE)
        .unwrap();
}

#[test]
fn finite_latest_history_does_not_invalidate_an_accepted_pending_lane() {
    let fixture = DeliveryFixture::new();
    let first = image_asset();
    fixture
        .service()
        .request_frame(&first, Time::ZERO, [1, 1])
        .expect("accepted oldest lane");
    // Stress finite history directly: fair dispatch would serve A before 64
    // unrelated completions. Seed only completed-history tags at the owning
    // seam; do not replace the one worker's in-flight identity or pending A.
    seed_completed_history(&fixture, 64);
    let (pending, latest) = {
        let queue = fixture.shared.queue.lock().expect("owned queue");
        (queue.frames.len(), queue.latest.len())
    };
    assert_eq!(pending, 1);
    assert!(latest <= 64, "history retains its bounded policy");
    let oldest = fixture.take_job();
    assert_eq!(oldest.asset.id, first.id);
    assert_eq!(oldest.sequence, 1);
    assert!(
        fixture.current(&oldest),
        "dispatching finite accepted work must retain its current identity"
    );
    complete_small_job(&fixture, &oldest);
    assert!(fixture.service().poll().is_none());
}

#[test]
fn latest_history_does_not_invalidate_a_captured_inflight_lane() {
    let fixture = DeliveryFixture::new();
    let first = image_asset();
    fixture
        .service()
        .request_frame(&first, Time::ZERO, [1, 1])
        .expect("accepted oldest lane");
    let captured = fixture.take_job();
    // Synthetic completed-history saturation preserves the real captured job
    // and single-in-flight invariant while testing its protection at pruning.
    seed_completed_history(&fixture, 63);
    assert!(fixture.current(&captured));
    let subsequent = image_asset();
    fixture
        .service()
        .request_frame(&subsequent, Time(10), [1, 1])
        .expect("accepted subsequent pending lane");
    let (pending, latest) = {
        let queue = fixture.shared.queue.lock().expect("owned queue");
        (queue.frames.len(), queue.latest.len())
    };
    assert_eq!(pending, 1);
    assert!(
        latest <= 64,
        "history remains bounded with one captured job"
    );
    assert!(
        fixture.current(&captured),
        "another lane cannot supersede a captured request in the same project"
    );
    complete_small_job(&fixture, &captured);
    let next = fixture.take_job();
    assert_eq!(next.asset.id, subsequent.id);
    complete_small_job(&fixture, &next);
}

fn seed_completed_history(fixture: &DeliveryFixture, count: usize) {
    let mut queue = fixture.shared.queue.lock().expect("owned queue");
    for _ in 0..count {
        let lane = Lane::Legacy {
            asset_id: image_asset().id,
            size: [1, 1],
        };
        queue.sequence = queue.sequence.wrapping_add(1);
        let sequence = queue.sequence;
        queue.latest.insert(lane, sequence);
        queue.prune_latest();
    }
}

#[test]
fn retiring_an_explicit_consumer_releases_its_byte_blocked_publication_without_discarding_others() {
    let (mut fixture, frame, assets) = filled_frame_mailbox();
    let consumer = PreviewConsumer::new();
    fixture
        .service()
        .request_preview(consumer, &assets[4], Time(9), LARGE_SIZE)
        .unwrap();
    let held = fixture.take_job();
    fixture.start_publication(held.clone(), frame.clone());
    fixture.expect_wait();
    fixture.service().retire_preview(consumer);
    assert!(!fixture.finish_publication().unwrap());
    assert!(!fixture.current(&held));
    assert_eq!(fixture.mailbox_usage(), (4, 4 * frame.byte_len()));
    assert_eq!(
        fixture.frame_ids(),
        assets[..4].iter().map(|asset| asset.id).collect::<Vec<_>>()
    );
}

#[test]
fn superseding_the_same_lane_invalidates_only_the_old_request() {
    let fixture = DeliveryFixture::new();
    let asset = image_asset();
    let old = fixture.request_job(&asset, Time::ZERO, [1, 1]);
    fixture
        .service()
        .request_frame(&asset, Time(100), [1, 1])
        .expect("new same-lane request");
    assert!(!fixture.current(&old));
    assert!(
        !publish_result(&fixture.shared, &old, Ok(small_frame(1)))
            .expect("old request is discarded")
    );
    assert!(fixture.service().poll().is_none());
    let new = fixture.take_job();
    assert!(new.sequence > old.sequence);
    assert!(fixture.current(&new));
    complete_small_job(&fixture, &new);
}

#[test]
fn project_cancel_invalidates_captured_jobs_and_their_results() {
    let fixture = DeliveryFixture::new();
    let asset = image_asset();
    let captured = fixture.request_job(&asset, Time::ZERO, [1, 1]);
    assert_ne!(fixture.service().cancel(), captured.generation);
    assert!(!fixture.current(&captured));
    assert!(
        !publish_result(&fixture.shared, &captured, Ok(small_frame(1)))
            .expect("late result is rejected")
    );
    assert!(
        !publish_result(&fixture.shared, &captured, Err(MediaError::Cancelled))
            .expect("cancelled decode has no result")
    );
    assert_eq!(fixture.mailbox_usage(), (0, 0));
    assert!(fixture.service().poll().is_none());
}

#[test]
fn five_distinct_large_results_wait_for_poll_without_discarding_prior_frames() {
    let (mut fixture, frame, assets) = filled_frame_mailbox();
    let fifth = fixture.request_job(&assets[4], Time(4), LARGE_SIZE);
    fixture.start_publication(fifth, frame.clone());
    fixture.expect_wait();
    assert_eq!(
        fixture.frame_ids(),
        assets[..4].iter().map(|asset| asset.id).collect::<Vec<_>>()
    );
    assert_eq!(fixture.mailbox_usage(), (4, 4 * frame.byte_len()));

    let polled = expect_frame(
        fixture.service().poll().expect("first retained frame"),
        assets[0].id,
        Time::ZERO,
        LARGE_SIZE,
    );
    assert!(Arc::ptr_eq(&polled.rgba, &frame.rgba));
    assert!(
        fixture
            .finish_publication()
            .expect("poll frees frame bytes")
    );
    assert_eq!(fixture.mailbox_usage(), (4, 4 * frame.byte_len()));
    for (index, asset) in assets.iter().enumerate().skip(1) {
        let polled = expect_frame(
            fixture.service().poll().expect("retained frame"),
            asset.id,
            Time(index as i64),
            LARGE_SIZE,
        );
        assert!(Arc::ptr_eq(&polled.rgba, &frame.rgba));
    }
    assert_eq!(fixture.mailbox_usage(), (0, 0));
    assert!(fixture.service().poll().is_none());
}

#[test]
fn cancellation_wakes_a_byte_blocked_publisher_and_rejects_its_result() {
    let (mut fixture, frame, assets) = filled_frame_mailbox();
    let blocked = fixture.request_job(&assets[4], Time(4), LARGE_SIZE);
    let generation = blocked.generation;
    fixture.start_publication(blocked, frame);
    fixture.expect_wait();
    assert_ne!(fixture.service().cancel(), generation);
    assert!(
        !fixture
            .finish_publication()
            .expect("cancelled publication returns")
    );
    assert_eq!(fixture.mailbox_usage(), (0, 0));
    assert!(fixture.service().poll().is_none());
}

#[test]
fn same_lane_supersession_wakes_a_byte_blocked_publisher() {
    let (mut fixture, frame, assets) = filled_frame_mailbox();
    let blocked = fixture.request_job(&assets[4], Time(4), LARGE_SIZE);
    let old_sequence = blocked.sequence;
    fixture.start_publication(blocked, frame.clone());
    fixture.expect_wait();
    fixture
        .service()
        .request_frame(&assets[4], Time(99), LARGE_SIZE)
        .expect("new request replaces blocked same-lane work");
    assert!(
        !fixture
            .finish_publication()
            .expect("superseded publication returns")
    );
    assert_eq!(
        fixture.frame_ids(),
        assets[..4].iter().map(|asset| asset.id).collect::<Vec<_>>()
    );
    assert_eq!(fixture.mailbox_usage(), (4, 4 * frame.byte_len()));

    expect_frame(
        fixture
            .service()
            .poll()
            .expect("poll frees one original result"),
        assets[0].id,
        Time::ZERO,
        LARGE_SIZE,
    );
    let replacement = fixture.take_job();
    assert!(replacement.sequence > old_sequence);
    assert_eq!(replacement.time, Time(99));
    assert!(
        publish_result(&fixture.shared, &replacement, Ok(frame.clone()))
            .expect("replacement fits after poll")
    );
    for (index, asset) in assets[1..4].iter().enumerate() {
        expect_frame(
            fixture.service().poll().expect("retained original result"),
            asset.id,
            Time((index + 1) as i64),
            LARGE_SIZE,
        );
    }
    expect_frame(
        fixture.service().poll().expect("latest replacement result"),
        assets[4].id,
        Time(99),
        LARGE_SIZE,
    );
    assert_eq!(fixture.mailbox_usage(), (0, 0));
}

#[test]
fn service_drop_wakes_a_byte_blocked_publisher_with_closed() {
    let (mut fixture, frame, assets) = filled_frame_mailbox();
    let blocked = fixture.request_job(&assets[4], Time(4), LARGE_SIZE);
    fixture.start_publication(blocked, frame);
    fixture.expect_wait();
    fixture.close();
    assert!(matches!(
        fixture.finish_publication(),
        Err(MediaError::Closed)
    ));
}

#[test]
fn newer_same_lane_result_replaces_old_frame_and_preserves_other_lane() {
    let fixture = DeliveryFixture::new();
    let first = image_asset();
    let other = image_asset();
    let old = fixture.request_job(&first, Time::ZERO, [1, 1]);
    assert!(publish_result(&fixture.shared, &old, Ok(small_frame(1))).expect("old result fits"));
    let other_job = fixture.request_job(&other, Time(20), [1, 1]);
    assert!(
        publish_result(&fixture.shared, &other_job, Ok(small_frame(2))).expect("other lane fits")
    );
    let new = fixture.request_job(&first, Time(30), [1, 1]);
    assert!(
        publish_result(&fixture.shared, &new, Ok(small_frame(3)))
            .expect("same-lane replacement fits")
    );
    assert_eq!(fixture.frame_ids(), vec![other.id, first.id]);
    assert_eq!(fixture.mailbox_usage(), (2, 8));
    let other_frame = expect_frame(
        fixture.service().poll().expect("other result survives"),
        other.id,
        Time(20),
        [1, 1],
    );
    assert_eq!(other_frame.rgba[0], 2);
    let new_frame = expect_frame(
        fixture.service().poll().expect("new result replaces old"),
        first.id,
        Time(30),
        [1, 1],
    );
    assert_eq!(new_frame.rgba[0], 3);
    assert_eq!(fixture.mailbox_usage(), (0, 0));
    assert!(fixture.service().poll().is_none());
}

#[test]
fn full_import_result_mailbox_applies_count_backpressure_without_evicting_imports() {
    let mut fixture = DeliveryFixture::new();
    let imports: Vec<_> = (0..64).map(|_| image_asset()).collect();
    let generation = fixture.service().generation();
    // Defensive synthetic saturation: the normal import producer permits at
    // most four queued Imported outcomes, so it cannot create this state.
    // The preview publisher must still preserve imports and its total budget.
    {
        let mut queue = fixture.shared.queue.lock().expect("owned queue");
        for asset in &imports {
            queue.events.push_back(MediaEvent::Imported {
                generation,
                assets: vec![asset.clone()],
                folders: Vec::new(),
                errors: Vec::new(),
            });
        }
    }
    let preview_asset = image_asset();
    let job = fixture.request_job(&preview_asset, Time::ZERO, [1, 1]);
    fixture.start_publication(job, small_frame(1));
    fixture.expect_wait();
    assert_eq!(fixture.mailbox_usage(), (64, 0));
    let retained_imports: Vec<_> = fixture
        .shared
        .queue
        .lock()
        .expect("owned queue")
        .events
        .iter()
        .filter_map(|event| match event {
            MediaEvent::Imported { assets, .. } => Some(assets[0].id),
            _ => None,
        })
        .collect();
    assert_eq!(
        retained_imports,
        imports.iter().map(|asset| asset.id).collect::<Vec<_>>()
    );

    for (index, asset) in imports.iter().enumerate() {
        let MediaEvent::Imported {
            generation: actual_generation,
            assets,
            ..
        } = fixture
            .service()
            .poll()
            .expect("every import result survives")
        else {
            panic!("preview cannot displace an import result");
        };
        assert_eq!(actual_generation, generation);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].id, asset.id);
        if index == 0 {
            assert!(
                fixture
                    .finish_publication()
                    .expect("poll frees one result slot")
            );
            assert_eq!(fixture.mailbox_usage(), (64, 4));
        }
    }
    expect_frame(
        fixture
            .service()
            .poll()
            .expect("new preview follows retained imports"),
        preview_asset.id,
        Time::ZERO,
        [1, 1],
    );
    assert_eq!(fixture.mailbox_usage(), (0, 0));
    assert!(fixture.service().poll().is_none());
}

#[test]
fn import_admission_respects_both_four_imports_and_the_total_result_count() {
    let fixture = DeliveryFixture::new();
    let generation = fixture.service().generation();
    {
        let mut queue = fixture.shared.queue.lock().expect("owned queue");
        assert!(queue.import_event_capacity());
        for _ in 0..3 {
            queue.events.push_back(MediaEvent::Imported {
                generation,
                assets: Vec::new(),
                folders: Vec::new(),
                errors: Vec::new(),
            });
        }
        assert!(
            queue.import_event_capacity(),
            "three imports leave one import slot"
        );
        queue.events.push_back(MediaEvent::Imported {
            generation,
            assets: Vec::new(),
            folders: Vec::new(),
            errors: Vec::new(),
        });
        assert!(
            !queue.import_event_capacity(),
            "the four-import policy remains bounded"
        );
        queue.events.clear();
    }
    let frame = small_frame(1);
    // Unlike the synthetic 64-Imported state, these 64 preview outcomes are
    // produced by real accepted requests and the actual publication seam.
    for _ in 0..63 {
        let asset = image_asset();
        let job = fixture.request_job(&asset, Time::ZERO, [1, 1]);
        assert!(
            publish_result(&fixture.shared, &job, Ok(frame.clone())).expect("preview result fits")
        );
    }
    assert!(
        fixture
            .shared
            .queue
            .lock()
            .expect("owned queue")
            .import_event_capacity()
    );
    let asset = image_asset();
    let job = fixture.request_job(&asset, Time::ZERO, [1, 1]);
    assert!(publish_result(&fixture.shared, &job, Ok(frame)).expect("64th preview result fits"));
    assert_eq!(fixture.mailbox_usage(), (64, 256));
    assert!(
        !fixture
            .shared
            .queue
            .lock()
            .expect("owned queue")
            .import_event_capacity(),
        "64 valid preview outcomes leave no total result slot for an import"
    );
    fixture
        .service()
        .poll()
        .expect("real poll frees a total result slot");
    assert!(
        fixture
            .shared
            .queue
            .lock()
            .expect("owned queue")
            .import_event_capacity()
    );
}
