//! Logical callback time only: no devices, sleeping, decoding or worker startup.
use super::*;
use editor_core::Playback;

const PLAYED_FRAMES: u64 = 48_000;
const AVAILABLE_POSITION: Time = Time(6_000_000);
const MARKER: [f32; 4] = [0.25, -0.25, 0.5, -0.5];

fn prime_state(state: &AudioState) {
    state.position.store(5_000_000, Ordering::Release);
    state.sample_rate.store(48_000, Ordering::Release);
    state.speed.store(1_f64.to_bits(), Ordering::Release);
    state.played.store(PLAYED_FRAMES, Ordering::Release);
    state
        .active_generation
        .store(state.generation.load(Ordering::Acquire), Ordering::Release);
}

struct LivenessFixture {
    state: Arc<AudioState>,
    ring: Ring,
    service: AudioService,
    _events: SyncSender<AudioEvent>,
}
impl LivenessFixture {
    fn new() -> Self {
        let state = Arc::new(AudioState::default());
        let ring = Ring::new(8, 2).unwrap();
        let mut callback = [1_f32; 4];
        ring.fill(&mut callback, &state, 2); // acknowledge the request generation
        assert_eq!(callback, [0.0; 4]);
        prime_state(&state);
        let (sender, events) = mpsc::sync_channel(8);
        let service = AudioService {
            state: Arc::clone(&state),
            processes: Processes::default(),
            events,
        };
        Self {
            state,
            ring,
            service,
            _events: sender,
        }
    }
}

#[test]
fn callback_clock_expires_at_the_half_second_floor_and_reports_one_notice() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 240);
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(599)),
        Some(AVAILABLE_POSITION),
    );
    assert!(!fixture.state.unavailable());
    assert!(fixture.service.poll().is_none());
    assert!(fixture.state.clock_at(Duration::from_millis(600)).is_none());
    assert!(fixture.state.unavailable());
    let notice = fixture
        .service
        .poll()
        .expect("callback expiry must be visible through public poll");
    assert!(!notice.trim().is_empty());
    assert!(fixture.state.clock_at(Duration::from_millis(601)).is_none());
    assert!(fixture.service.poll().is_none());
    assert!(fixture.service.position().is_none());
}

#[test]
fn callback_clock_allows_four_actual_periods_for_a_one_second_output_buffer() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 48_000);
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(4099)),
        Some(AVAILABLE_POSITION),
    );
    assert!(!fixture.state.unavailable());
    assert!(
        fixture
            .state
            .clock_at(Duration::from_millis(4100))
            .is_none()
    );
    assert!(fixture.state.unavailable());
}

#[test]
fn refreshed_callback_extends_health_past_the_previous_deadline() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 240);
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(500)),
        Some(AVAILABLE_POSITION),
    );
    fixture
        .state
        .record_callback_at(Duration::from_millis(550), 240);
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(600)),
        Some(AVAILABLE_POSITION),
    );
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(1049)),
        Some(AVAILABLE_POSITION),
    );
    assert!(
        fixture
            .state
            .clock_at(Duration::from_millis(1050))
            .is_none()
    );
}

#[test]
fn paused_callback_clock_does_not_latch_failure_from_an_old_heartbeat() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 240);
    fixture.service.stop();
    assert!(fixture.state.clock_at(Duration::from_secs(3600)).is_none());
    assert!(!fixture.state.unavailable());
    assert!(fixture.service.poll().is_none());
}

#[test]
fn an_unarmed_pure_clock_fixture_does_not_invent_native_stream_failure() {
    let state = AudioState::default();
    prime_state(&state);
    // No callback or output-watch startup occurred. Existing pure clock
    // fixtures remain separate from an actual native stream's liveness.
    assert_eq!(
        state.clock_at(Duration::from_secs(3600)),
        Some(AVAILABLE_POSITION)
    );
    assert!(!state.unavailable());
}

#[test]
fn callback_timeout_restores_real_playback_wall_clock_advancement() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 240);
    let now = Instant::now();
    let duration = Time(20_000_000);
    let mut playback = Playback::default();
    playback.play();
    assert!(playback.tick_with_audio(
        now,
        duration,
        fixture.state.clock_at(Duration::from_millis(599)),
    ));
    assert_eq!(playback.position, AVAILABLE_POSITION);
    assert!(playback.tick_with_audio(
        now + Duration::from_millis(100),
        duration,
        fixture.state.clock_at(Duration::from_millis(600)),
    ));
    assert_eq!(playback.position, Time(6_100_000));
    playback.tick_with_audio(
        now + Duration::from_millis(200),
        duration,
        fixture.state.clock_at(Duration::from_millis(700)),
    );
    assert_eq!(playback.position, Time(6_200_000));
    assert!(playback.playing);
}

#[test]
fn callback_timeout_rejects_late_pcm_activation_and_future_sync() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 240);
    assert!(fixture.state.clock_at(Duration::from_millis(600)).is_none());
    let generation = fixture.state.generation.load(Ordering::Acquire);
    fixture
        .state
        .active_generation
        .store(generation, Ordering::Release);
    assert!(fixture.ring.push(generation, PLAYED_FRAMES, &MARKER));
    let mut enabled = false;
    assert_eq!(
        publish_decoded(
            &fixture.ring,
            &fixture.state,
            generation,
            PLAYED_FRAMES,
            &MARKER,
            &mut enabled,
            || {},
        ),
        DecodedPublication::Cancelled,
    );
    assert!(!enabled);
    let mut callback = [1_f32; 4];
    fixture.ring.fill(&mut callback, &fixture.state, 2);
    assert_eq!(callback, [0.0; 4]);
    assert_eq!(fixture.state.played.load(Ordering::Acquire), PLAYED_FRAMES);
    fixture
        .state
        .record_callback_at(Duration::from_millis(700), 240);
    assert!(fixture.state.clock_at(Duration::from_millis(800)).is_none());
    let project = Arc::new(Project::new("Owned timeout pending fixture"));
    let reference = Arc::downgrade(&project);
    fixture.service.sync(project, Time::ZERO, true, 1.0);
    assert!(fixture.state.pending.lock().unwrap().is_none());
    assert!(reference.upgrade().is_none());
}

#[test]
fn reset_callback_can_refresh_the_heartbeat_before_the_new_epoch_activates() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 240);
    fixture.state.active_generation.store(0, Ordering::Release);
    let generation = fixture.state.generation.fetch_add(1, Ordering::AcqRel) + 1;
    assert!(
        fixture
            .state
            .clock_at(Duration::from_millis(1000))
            .is_none()
    );
    assert!(!fixture.state.unavailable());
    let mut callback = [1_f32; 4];
    fixture.ring.fill(&mut callback, &fixture.state, 2); // acknowledge the seek reset
    fixture.state.played.store(0, Ordering::Release);
    fixture
        .state
        .record_callback_at(Duration::from_millis(1100), 240);
    assert!(
        fixture
            .state
            .clock_at(Duration::from_millis(1200))
            .is_none()
    );
    assert!(!fixture.state.unavailable());
    fixture
        .state
        .active_generation
        .store(generation, Ordering::Release);
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(1599)),
        Some(Time(5_000_000)),
    );
    assert!(!fixture.state.unavailable());
}

#[test]
fn a_smaller_buffer_cannot_expire_a_live_clock_between_callback_stores() {
    let fixture = LivenessFixture::new();
    fixture
        .state
        .record_callback_at(Duration::from_millis(100), 48_000);
    fixture
        .state
        .record_callback_at_before_heartbeat(Duration::from_millis(1100), 240, || {
            assert_eq!(
                fixture.state.clock_at(Duration::from_millis(1100)),
                Some(AVAILABLE_POSITION),
            );
            assert!(!fixture.state.unavailable());
        });
    // Retaining the largest observed period is conservative for this stream
    // lifetime, including after later buffers become smaller.
    assert_eq!(
        fixture.state.clock_at(Duration::from_millis(5099)),
        Some(AVAILABLE_POSITION),
    );
    assert!(
        fixture
            .state
            .clock_at(Duration::from_millis(5100))
            .is_none()
    );
}
