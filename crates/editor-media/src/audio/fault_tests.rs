//! Device-free regressions through the same seam as the native error callback.
use super::*;
use cpal::{Error as OutputError, ErrorKind};
use editor_core::Playback;

const MARKER: [f32; 4] = [0.25, -0.25, 0.5, -0.5];
const PLAYED: u64 = 48_000;

struct FaultFixture {
    state: Arc<AudioState>,
    ring: Ring,
    errors: SyncSender<AudioEvent>,
    events: Receiver<AudioEvent>,
    wake: Wake,
}
impl FaultFixture {
    fn new() -> Self {
        let state = Arc::new(AudioState::default());
        let ring = Ring::new(8, 2).unwrap();
        let mut callback = [1_f32; 4];
        ring.fill(&mut callback, &state, 2); // acknowledge the initial generation
        assert_eq!(callback, [0.0; 4]);
        state.position.store(5_000_000, Ordering::Release);
        state.sample_rate.store(48_000, Ordering::Release);
        state.speed.store(1_f64.to_bits(), Ordering::Release);
        state.played.store(PLAYED, Ordering::Release);
        state
            .active_generation
            .store(state.generation.load(Ordering::Acquire), Ordering::Release);
        let (errors, events) = mpsc::sync_channel(8);
        Self {
            state,
            ring,
            errors,
            events,
            wake: Arc::new(|| {}),
        }
    }

    fn report(&self, kind: ErrorKind) {
        handle_output_error(
            &self.state,
            OutputError::new(kind),
            &self.errors,
            &self.wake,
        );
    }
}

#[test]
fn fatal_output_error_invalidates_an_available_device_clock() {
    let fixture = FaultFixture::new();
    assert_eq!(fixture.state.clock(), Some(Time(6_000_000)));
    fixture.report(ErrorKind::DeviceNotAvailable);
    assert!(
        fixture.state.clock().is_none(),
        "a stopped output stream must not remain a permanently available clock"
    );
    assert!(matches!(
        fixture.events.try_recv().unwrap(),
        AudioEvent::Output(error) if error.kind() == ErrorKind::DeviceNotAvailable
    ));
    let healthy_replacement = FaultFixture::new();
    assert_eq!(healthy_replacement.state.clock(), Some(Time(6_000_000)));
}

#[test]
fn fatal_output_error_invalidates_clock_when_eight_notices_already_fill_the_queue() {
    let fixture = FaultFixture::new();
    for _ in 0..8 {
        assert!(
            fixture
                .errors
                .try_send(AudioEvent::Message("owned queued notice".into()))
                .is_ok()
        );
    }
    fixture.report(ErrorKind::StreamInvalidated);
    assert!(
        fixture.state.clock().is_none(),
        "clock safety cannot depend on space in the notification queue"
    );
    assert_eq!(fixture.events.try_iter().count(), 8);
}

#[test]
fn fatal_output_error_rejects_late_activation_and_already_queued_marker_pcm() {
    let fixture = FaultFixture::new();
    fixture.report(ErrorKind::BackendError);

    // An in-flight publisher can still execute its previously checked store.
    // A matching request stamp cannot repair a failed output stream.
    let generation = fixture.state.generation.load(Ordering::Acquire);
    fixture
        .state
        .active_generation
        .store(generation, Ordering::Release);
    assert!(fixture.ring.push(generation, PLAYED, &MARKER));
    assert!(
        fixture.state.clock().is_none(),
        "late activation cannot restore a failed stream's clock"
    );
    let mut callback = [1_f32; 4];
    fixture.ring.fill(&mut callback, &fixture.state, 2);
    assert_eq!(callback, [0.0; 4]);
    assert_eq!(fixture.state.played.load(Ordering::Acquire), PLAYED);
    assert!(fixture.state.clock().is_none());
}

#[test]
fn fatal_output_error_cancels_decode_publication_after_the_request_guard() {
    let fixture = FaultFixture::new();
    let generation = fixture.state.generation.load(Ordering::Acquire);
    fixture.state.active_generation.store(0, Ordering::Release);
    fixture.state.played.store(0, Ordering::Release);
    let mut enabled = false;
    let publication = publish_decoded(
        &fixture.ring,
        &fixture.state,
        generation,
        0,
        &MARKER,
        &mut enabled,
        || fixture.report(ErrorKind::DeviceNotAvailable),
    );
    assert_eq!(publication, DecodedPublication::Cancelled);
    assert!(!enabled);
    assert_eq!(fixture.ring.available(), 0);
    assert!(fixture.state.clock().is_none());
    let mut callback = [1_f32; 4];
    fixture.ring.fill(&mut callback, &fixture.state, 2);
    assert_eq!(callback, [0.0; 4]);
    assert_eq!(fixture.state.played.load(Ordering::Acquire), 0);
}

#[test]
fn fatal_output_error_allows_real_playback_to_resume_from_the_last_audio_anchor() {
    let fixture = FaultFixture::new();
    let mut playback = Playback::default();
    let now = Instant::now();
    let duration = Time(20_000_000);
    playback.play();
    assert!(playback.tick_with_audio(now, duration, fixture.state.clock()));
    assert_eq!(playback.position, Time(6_000_000));
    fixture.report(ErrorKind::DeviceNotAvailable);
    assert!(playback.tick_with_audio(
        now + Duration::from_millis(200),
        duration,
        fixture.state.clock(),
    ));
    assert_eq!(playback.position, Time(6_200_000));
    assert!(playback.playing);
    playback.tick_with_audio(
        now + Duration::from_millis(300),
        duration,
        fixture.state.clock(),
    );
    assert_eq!(playback.position, Time(6_300_000));
}

#[test]
fn fatal_output_error_rejects_future_sync_without_retaining_its_project() {
    let FaultFixture {
        state,
        ring: _,
        errors,
        events,
        wake,
    } = FaultFixture::new();
    let service = AudioService {
        state: Arc::clone(&state),
        processes: Processes::default(),
        events,
    };
    handle_output_error(
        &state,
        OutputError::new(ErrorKind::StreamInvalidated),
        &errors,
        &wake,
    );
    let project = Arc::new(Project::new("Owned no-device pending fixture"));
    let reference = Arc::downgrade(&project);
    service.sync(project, Time::ZERO, true, 1.0);
    assert!(
        state.pending.lock().unwrap().is_none(),
        "a failed worker cannot accept a replacement project for later playback"
    );
    assert!(
        reference.upgrade().is_none(),
        "the failed service must release the submitted project snapshot"
    );
    assert!(service.position().is_none());
}

#[test]
fn recoverable_output_notifications_preserve_clock_and_pcm_progress() {
    for kind in [
        ErrorKind::DeviceChanged,
        ErrorKind::RealtimeDenied,
        ErrorKind::Xrun,
    ] {
        let fixture = FaultFixture::new();
        handle_output_error(
            &fixture.state,
            OutputError::with_message(kind, "owned recoverable output notification"),
            &fixture.errors,
            &fixture.wake,
        );
        assert_eq!(fixture.state.clock(), Some(Time(6_000_000)));
        let generation = fixture.state.generation.load(Ordering::Acquire);
        assert!(fixture.ring.push(generation, PLAYED, &MARKER));
        let mut callback = [1_f32; 4];
        fixture.ring.fill(&mut callback, &fixture.state, 2);
        assert_eq!(callback, MARKER);
        assert_eq!(fixture.state.played.load(Ordering::Acquire), PLAYED + 2);
        assert!(fixture.state.clock().unwrap() > Time(6_000_000));
        assert!(matches!(
            fixture.events.try_recv().unwrap(),
            AudioEvent::Output(error) if error.kind() == kind
        ));
    }
}

// Additional lifecycle coverage; the seven initial red/green regressions above
// remain unchanged so their preserved failure evidence has a fixed reference.

#[test]
fn pinned_output_error_categories_apply_the_documented_current_stream_policy() {
    let cases = [
        (ErrorKind::DeviceBusy, true),
        (ErrorKind::DeviceChanged, false),
        (ErrorKind::DeviceNotAvailable, true),
        (ErrorKind::HostUnavailable, true),
        (ErrorKind::InvalidInput, true),
        (ErrorKind::PermissionDenied, true),
        (ErrorKind::RealtimeDenied, false),
        (ErrorKind::ResourceExhausted, true),
        (ErrorKind::StreamInvalidated, true),
        (ErrorKind::UnsupportedConfig, true),
        (ErrorKind::UnsupportedOperation, true),
        (ErrorKind::Xrun, false),
        (ErrorKind::BackendError, true),
        (ErrorKind::Other, true),
    ];
    for (kind, unavailable) in cases {
        let fixture = FaultFixture::new();
        fixture.report(kind);
        assert_eq!(fixture.state.unavailable(), unavailable, "{kind:?}");
        assert_eq!(fixture.state.clock().is_none(), unavailable, "{kind:?}");
        assert!(matches!(
            fixture.events.try_recv().unwrap(),
            AudioEvent::Output(error) if error.kind() == kind
        ));
    }
}

#[test]
fn saturated_notices_preserve_one_coalesced_fatal_notice_for_public_poll() {
    let FaultFixture {
        state,
        ring: _,
        errors,
        events,
        wake,
    } = FaultFixture::new();
    let service = AudioService {
        state: Arc::clone(&state),
        processes: Processes::default(),
        events,
    };
    for index in 0..8 {
        assert!(
            errors
                .try_send(AudioEvent::Message(format!("owned queued notice {index}")))
                .is_ok()
        );
    }
    for kind in [ErrorKind::DeviceNotAvailable, ErrorKind::StreamInvalidated] {
        handle_output_error(&state, OutputError::new(kind), &errors, &wake);
    }
    assert!(service.position().is_none());
    for index in 0..8 {
        assert_eq!(service.poll(), Some(format!("owned queued notice {index}")));
    }
    assert_eq!(
        service.poll().as_deref(),
        Some("Audio output is unavailable")
    );
    assert!(service.poll().is_none());
    assert!(service.poll().is_none());
}

#[test]
fn fatal_notification_just_before_park_retains_its_wake_token() {
    let state = Arc::new(AudioState::default());
    let reached_hook = Arc::new(AtomicBool::new(false));
    let release_hook = Arc::new(AtomicBool::new(false));
    let worker_state = Arc::clone(&state);
    let worker_reached = Arc::clone(&reached_hook);
    let worker_release = Arc::clone(&release_hook);
    let (completed, completion) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let _ = worker_state.worker.set(thread::current());
        let result = worker_state.wait_request_before_park(|| {
            // This hook contains only atomics and the spin hint. Calling a
            // channel, printing, sleeping or locking here could consume the
            // worker's parking token and invalidate the controlled ordering.
            worker_reached.store(true, Ordering::Release);
            while !worker_release.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
        });
        let _ = completed.send(result.is_ok_and(|request| request.is_none()));
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !reached_hook.load(Ordering::Acquire) && Instant::now() < deadline {
        std::hint::spin_loop();
    }
    let reached = reached_hook.load(Ordering::Acquire);
    let (errors, _events) = mpsc::sync_channel(8);
    let wake: Wake = Arc::new(|| {});
    if reached {
        // The registered worker is held before its actual park call. Fatal
        // notification completes before the release; no parked-state claim.
        handle_output_error(
            &state,
            OutputError::new(ErrorKind::DeviceNotAvailable),
            &errors,
            &wake,
        );
    } else {
        state.stopped.store(true, Ordering::Release);
        worker.thread().unpark();
    }
    release_hook.store(true, Ordering::Release);
    let outcome = completion.recv_timeout(Duration::from_secs(2));
    if outcome.is_err() {
        // Only this test's thread is released during failure cleanup. Setting
        // shutdown makes the real request predicate exit after the fallback
        // unpark instead of leaving a failed test's waiter behind.
        state.stopped.store(true, Ordering::Release);
        release_hook.store(true, Ordering::Release);
        worker.thread().unpark();
    }
    let joined = worker.join();
    assert!(
        reached,
        "the owned worker must reach the controlled pre-park hook"
    );
    assert!(
        matches!(outcome, Ok(true)),
        "a pre-park fatal notification must not be lost"
    );
    assert!(joined.is_ok());
}

#[test]
fn pending_request_before_worker_registration_is_taken_without_retargeting() {
    let state = AudioState::default();
    assert!(state.worker.get().is_none());
    let project = Arc::new(Project::new("Owned early pending fixture"));
    let project_id = project.id;
    let reference = Arc::downgrade(&project);
    *state.pending.lock().unwrap() = Some(AudioRequest {
        generation: state.generation.load(Ordering::Acquire),
        project,
        position: Time(25_000),
        playing: true,
        speed: 0.5,
    });
    let request = state.wait_request().unwrap().unwrap();
    assert_eq!(request.project.id, project_id);
    assert!(Arc::ptr_eq(&request.project, &reference.upgrade().unwrap()));
    assert_eq!(request.position, Time(25_000));
    assert_eq!(request.speed, 0.5);
    assert!(request.playing);
    assert!(state.pending.lock().unwrap().is_none());
    assert!(state.worker.get().is_none());
    drop(request);
    assert!(reference.upgrade().is_none());
}

#[test]
fn shutdown_or_failure_before_worker_registration_drains_the_pending_project() {
    for fatal in [false, true] {
        let state = AudioState::default();
        assert!(state.worker.get().is_none());
        let project = Arc::new(Project::new("Owned unavailable pending fixture"));
        let reference = Arc::downgrade(&project);
        *state.pending.lock().unwrap() = Some(AudioRequest {
            generation: state.generation.load(Ordering::Acquire),
            project,
            position: Time::ZERO,
            playing: true,
            speed: 1.0,
        });
        if fatal {
            let (errors, _events) = mpsc::sync_channel(8);
            let wake: Wake = Arc::new(|| {});
            handle_output_error(
                &state,
                OutputError::new(ErrorKind::DeviceNotAvailable),
                &errors,
                &wake,
            );
        } else {
            state.stopped.store(true, Ordering::Release);
            state.notify();
        }
        assert!(state.wait_request().unwrap().is_none());
        assert!(state.pending.lock().unwrap().is_none());
        assert!(reference.upgrade().is_none());
        assert!(state.worker.get().is_none());
    }
}
