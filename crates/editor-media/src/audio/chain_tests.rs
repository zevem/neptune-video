//! The voice chain: its strings, the probe that finds which kinds FFmpeg
//! runs, and readback through the mixer of what each kind does to sound the
//! test wrote and of the delay that is taken off a voice's start.
use super::level_tests::{decoder_available, project_of};
use super::*;
use chain::{
    ONE_CHANNEL_BIT, bit_of, chain, effects, latency_frames, left_out, probe, probe_strings, runs,
    speed_stage,
};
use editor_core::{Asset, Clip, Effect, EffectKind, MediaKind, ParamKey, ParamValue};
use std::{
    f64::consts::TAU,
    path::Path,
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};

/// The rate the fixtures are written at.
const SOURCE_RATE: u32 = 48_000;
const PLAIN: &str = "aresample=48000:async=1:first_pts=0,asetrate=48000,aresample=48000";

fn effect(kind: EffectKind, params: &[(ParamKey, f32)]) -> Effect {
    Effect {
        params: (params.iter())
            .map(|&(key, value)| ParamValue { key, value })
            .collect(),
        ..Effect::new(kind)
    }
}

/// A clip of sound that is never opened, for the strings.
fn clip_with(effects: Vec<Effect>) -> Clip {
    let asset = Asset::new(
        "never-opened.wav".into(),
        MediaKind::Audio,
        Time::from_seconds(1.0),
    );
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.effects = effects;
    clip
}

fn string_of(effects: Vec<Effect>) -> String {
    chain(
        &clip_with(effects),
        1.0,
        1.0,
        48_000,
        2,
        &Filters::default(),
    )
    .unwrap()
}

/// A stereo 32-bit float WAV at 48 kHz.
fn stereo(path: &Path, left: &[f32], right: &[f32]) -> Asset {
    assert_eq!(left.len(), right.len());
    let data_bytes = left.len() as u32 * 8;
    let mut wave = Vec::with_capacity(44 + data_bytes as usize);
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16_u32.to_le_bytes());
    wave.extend_from_slice(&3_u16.to_le_bytes()); // IEEE float
    wave.extend_from_slice(&2_u16.to_le_bytes());
    wave.extend_from_slice(&SOURCE_RATE.to_le_bytes());
    wave.extend_from_slice(&(SOURCE_RATE * 8).to_le_bytes());
    wave.extend_from_slice(&8_u16.to_le_bytes());
    wave.extend_from_slice(&32_u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&data_bytes.to_le_bytes());
    for (left, right) in left.iter().zip(right) {
        wave.extend_from_slice(&left.to_le_bytes());
        wave.extend_from_slice(&right.to_le_bytes());
    }
    std::fs::write(path, wave).unwrap();
    Asset::new(
        path.to_path_buf(),
        MediaKind::Audio,
        Time::from_seconds(left.len() as f64 / f64::from(SOURCE_RATE)),
    )
}

fn tone(hertz: f64, amplitude: f64, seconds: f64) -> Vec<f32> {
    (0..(seconds * f64::from(SOURCE_RATE)) as usize)
        .map(|frame| {
            (amplitude * (TAU * hertz * frame as f64 / f64::from(SOURCE_RATE)).sin()) as f32
        })
        .collect()
}

/// `seconds` from the project's start through a mixer at `rate` in
/// `channels`, asked for chunk by chunk as playback asks, and the mixer.
fn mixed(
    project: &Project,
    rate: u32,
    channels: usize,
    filters: Filters,
    seconds: f64,
) -> (Vec<f32>, Mixer) {
    project.validate().unwrap();
    let mut mixer = Mixer::new(
        Arc::new(project.clone()),
        1.0,
        rate,
        channels,
        Processes::default(),
    )
    .with_filters(Arc::new(filters));
    let chunks = (seconds * f64::from(rate) / CHUNK_FRAMES as f64).ceil() as usize;
    let mut samples = Vec::new();
    for chunk in 0..chunks {
        let start = Time::from_seconds((chunk * CHUNK_FRAMES) as f64 / f64::from(rate));
        samples.extend(mixer.chunk(start, CHUNK_FRAMES).unwrap());
    }
    (samples, mixer)
}

/// Both channels of [`mixed`] in two.
fn heard_at(
    project: &Project,
    rate: u32,
    filters: Filters,
    seconds: f64,
) -> (Vec<f32>, Vec<f32>, Mixer) {
    let (samples, mixer) = mixed(project, rate, 2, filters, seconds);
    let (left, right) = samples
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .unzip();
    (left, right, mixer)
}

/// The left channel of a clip of `asset` with `effects`, at 48 kHz.
fn heard(asset: &Asset, effects: Vec<Effect>, seconds: f64) -> Vec<f32> {
    let mut clip = Clip::from_asset(asset, Time::ZERO);
    clip.effects = effects;
    heard_at(
        &project_of(&[asset], vec![clip]),
        SOURCE_RATE,
        Filters::default(),
        seconds,
    )
    .0
}

/// Level in dBFS RMS of `samples` from `from` to `to` seconds at 48 kHz.
fn level(samples: &[f32], from: f64, to: f64) -> f64 {
    let at = |seconds: f64| (seconds * f64::from(SOURCE_RATE)) as usize;
    let span = &samples[at(from)..at(to)];
    let power = span.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / span.len() as f64;
    10.0 * power.max(1.0e-20).log10()
}

#[test]
fn a_clip_without_audio_effects_gets_exactly_the_speed_stage() {
    let filters = Filters::default();
    let plain = clip_with(vec![]);
    assert_eq!(chain(&plain, 1.0, 1.0, 48_000, 2, &filters).unwrap(), PLAIN);
    assert_eq!(
        chain(&plain, 2.0, 0.5, 44_100, 2, &filters).unwrap(),
        "aresample=44100:async=1:first_pts=0,asetrate=44100,aresample=44100"
    );
    // The changed rate is rounded as it always was: 44100 x 1.5 x 0.3.
    assert_eq!(
        chain(&plain, 0.3, 1.5, 44_100, 2, &filters).unwrap(),
        "aresample=44100:async=1:first_pts=0,asetrate=19845,aresample=44100"
    );
    assert_eq!(effects(&plain, 48_000, 2, &filters), "");
    assert_eq!(latency_frames(&plain, 48_000, 2, &filters), 0);
    assert_eq!(left_out(&plain, 2, &filters), 0);

    for (clip_speed, transport) in [(16.0, 16.0), (0.001, 0.001)] {
        let error = chain(&plain, clip_speed, transport, 48_000, 2, &filters).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Clip and preview speeds exceed the audio resampling range"
        );
    }
}

/// A kind, the values stored for it and the filter it is written as.
type Case = (EffectKind, &'static [(ParamKey, f32)], &'static str);

#[test]
fn each_kind_writes_its_filter_at_its_defaults_and_at_its_extremes() {
    use EffectKind as K;
    use ParamKey as P;
    let after = |effects: Vec<Effect>| {
        let string = string_of(effects);
        string
            .strip_prefix(&format!("{PLAIN},"))
            .unwrap_or_else(|| panic!("{string}"))
            .to_owned()
    };
    let cases: [Case; 19] = [
        (K::HighPass, &[], "highpass=f=80.000"),
        (K::HighPass, &[(P::Frequency, 20.0)], "highpass=f=20.000"),
        (
            K::HighPass,
            &[(P::Frequency, 2_000.0)],
            "highpass=f=2000.000",
        ),
        (K::EqBand, &[], "equalizer=f=1000.000:t=o:w=1.000:g=0.000"),
        (
            K::EqBand,
            &[(P::Frequency, 20.0), (P::Gain, -24.0), (P::Width, 0.1)],
            "equalizer=f=20.000:t=o:w=0.100:g=-24.000",
        ),
        (
            K::EqBand,
            &[(P::Frequency, 20_000.0), (P::Gain, 24.0), (P::Width, 4.0)],
            "equalizer=f=20000.000:t=o:w=4.000:g=24.000",
        ),
        // -18 dB is 0.125893 and no make-up is 1.
        (
            K::Compressor,
            &[],
            "acompressor=threshold=0.125893:ratio=3.000:attack=10:release=150:makeup=1.000000",
        ),
        (
            K::Compressor,
            &[(P::Threshold, -60.0), (P::Amount, 1.0), (P::Gain, 0.0)],
            "acompressor=threshold=0.001000:ratio=1.000:attack=10:release=150:makeup=1.000000",
        ),
        (
            K::Compressor,
            &[(P::Threshold, 0.0), (P::Amount, 20.0), (P::Gain, 24.0)],
            "acompressor=threshold=1.000000:ratio=20.000:attack=10:release=150:makeup=15.848932",
        ),
        (K::Limiter, &[], "alimiter=limit=0.891251:level=false"),
        (
            K::Limiter,
            &[(P::Threshold, -24.0)],
            "alimiter=limit=0.063096:level=false",
        ),
        (
            K::Limiter,
            &[(P::Threshold, 0.0)],
            "alimiter=limit=1.000000:level=false",
        ),
        // Amount 0..1 is nr 6..36 and the floor is nf as it is. No `tn`.
        (K::Denoise, &[], "afftdn=nr=18.000:nf=-40.000"),
        (
            K::Denoise,
            &[(P::Amount, 0.0), (P::Threshold, -80.0)],
            "afftdn=nr=6.000:nf=-80.000",
        ),
        (
            K::Denoise,
            &[(P::Amount, 1.0), (P::Threshold, -20.0)],
            "afftdn=nr=36.000:nf=-20.000",
        ),
        (
            K::CenterCut,
            &[],
            "aformat=channel_layouts=stereo,pan=stereo|c0=c0-1.0000*c1|c1=c1-1.0000*c0",
        ),
        (
            K::CenterCut,
            &[(P::Amount, 0.0)],
            "aformat=channel_layouts=stereo,pan=stereo|c0=c0-0.0000*c1|c1=c1-0.0000*c0",
        ),
        (
            K::CenterCut,
            &[(P::Amount, 0.25)],
            "aformat=channel_layouts=stereo,pan=stereo|c0=c0-0.2500*c1|c1=c1-0.2500*c0",
        ),
        (
            K::EqBand,
            &[(P::Gain, 3.0), (P::Frequency, 3_500.0)],
            "equalizer=f=3500.000:t=o:w=1.000:g=3.000",
        ),
    ];
    for (kind, params, expected) in cases {
        let effect = effect(kind, params);
        assert!(effect.validate(), "{kind:?} {params:?}");
        assert_eq!(after(vec![effect]), expected, "{kind:?} {params:?}");
    }
    // What the acompressor and the alimiter of FFmpeg 8.1.2 take: a
    // threshold from 0.000976563, a make-up from 1 to 64, a limit from 0.0625.
    assert!(10_f64.powf(-60.0 / 20.0) >= 0.000_976_563);
    assert!(10_f64.powf(24.0 / 20.0) <= 64.0);
    assert!(10_f64.powf(-24.0 / 20.0) >= 0.0625);
}

#[test]
fn a_frequency_stays_under_half_the_output_rate() {
    let high = clip_with(vec![
        effect(EffectKind::EqBand, &[(ParamKey::Frequency, 20_000.0)]),
        effect(EffectKind::HighPass, &[(ParamKey::Frequency, 2_000.0)]),
    ]);
    let filters = Filters::default();
    // A headset at 16 kHz: a band at 20 kHz would be past what it carries.
    assert_eq!(
        effects(&high, 16_000, 2, &filters),
        "equalizer=f=7200.000:t=o:w=1.000:g=0.000,highpass=f=2000.000"
    );
    assert_eq!(
        effects(&high, 44_100, 2, &filters),
        "equalizer=f=19845.000:t=o:w=1.000:g=0.000,highpass=f=2000.000"
    );
    assert_eq!(
        effects(&high, 48_000, 2, &filters),
        "equalizer=f=20000.000:t=o:w=1.000:g=0.000,highpass=f=2000.000"
    );
}

#[test]
fn the_chain_keeps_stack_order_and_leaves_out_what_is_disabled_or_missing() {
    use EffectKind as K;
    let mut disabled = effect(K::Compressor, &[]);
    disabled.enabled = false;
    let stack = vec![
        effect(K::Limiter, &[]),
        effect(K::HighPass, &[]),
        disabled,
        effect(K::EqBand, &[(ParamKey::Frequency, 120.0)]),
        effect(K::Denoise, &[]),
        // The speed stage is `Pitch`; it is no filter after it.
        effect(K::Pitch, &[(ParamKey::Amount, 3.0)]),
        effect(K::EqBand, &[(ParamKey::Frequency, 8_000.0)]),
    ];
    let clip = clip_with(stack);
    let all = Filters::default();
    let whole = "alimiter=limit=0.891251:level=false,highpass=f=80.000,\
                 equalizer=f=120.000:t=o:w=1.000:g=0.000,afftdn=nr=18.000:nf=-40.000,\
                 equalizer=f=8000.000:t=o:w=1.000:g=0.000";
    assert_eq!(effects(&clip, 48_000, 2, &all), whole);
    // Three semitones up at speed 1: 48000 x 2^(3/12), and the tempo back.
    let pitched =
        "aresample=48000:async=1:first_pts=0,asetrate=57082,aresample=48000,atempo=0.840896";
    assert_eq!(
        chain(&clip, 1.0, 1.0, 48_000, 2, &all).unwrap(),
        format!("{pitched},{whole}")
    );
    assert_eq!(left_out(&clip, 2, &all), 0);
    // 5 ms of the limiter and 25 ms of the noise reduction, in frames.
    assert_eq!(latency_frames(&clip, 48_000, 2, &all), 1_440);
    assert_eq!(latency_frames(&clip, 44_100, 2, &all), 1_323);
    assert_eq!(latency_frames(&clip, 96_000, 2, &all), 2_880);

    // An FFmpeg without afftdn and without the equalizer: neither is in the
    // chain, and neither's delay is taken off the voice.
    let some = Filters::without(&[K::Denoise, K::EqBand, K::Compressor]);
    assert_eq!(
        effects(&clip, 48_000, 2, &some),
        "alimiter=limit=0.891251:level=false,highpass=f=80.000"
    );
    assert!(
        (chain(&clip, 1.0, 1.0, 48_000, 2, &some).unwrap()).starts_with(&format!("{pitched},"))
    );
    assert_eq!(latency_frames(&clip, 48_000, 2, &some), 240);
    // The compressor is missing and disabled: nothing went without it.
    assert_eq!(
        left_out(&clip, 2, &some),
        bit_of(K::Denoise) | bit_of(K::EqBand)
    );

    let mut none = clip.clone();
    for effect in &mut none.effects {
        effect.enabled = false;
    }
    assert_eq!(chain(&none, 1.0, 1.0, 48_000, 2, &all).unwrap(), PLAIN);
    assert_eq!(latency_frames(&none, 48_000, 2, &all), 0);
}

#[test]
fn the_probe_runs_a_control_and_then_each_kind_at_its_defaults() {
    let mut ran = Vec::new();
    let filters = probe(|filter| {
        ran.push(filter.to_owned());
        true
    });
    assert_eq!(
        ran,
        [
            "anull",
            "highpass=f=80.000",
            "equalizer=f=1000.000:t=o:w=1.000:g=0.000",
            "acompressor=threshold=0.125893:ratio=3.000:attack=10:release=150:makeup=1.000000",
            "alimiter=limit=0.891251:level=false",
            "afftdn=nr=18.000:nf=-40.000",
            "atempo=1.5",
            "aformat=channel_layouts=stereo,pan=stereo|c0=c0-1.0000*c1|c1=c1-1.0000*c0",
        ]
    );
    assert_eq!(ran[1..], probe_strings());
    assert!(filters.missing().is_empty() && !filters.unknown());
    assert!(filters.unsaid(0).is_empty());
}

#[test]
fn a_kind_whose_run_fails_is_the_only_one_missing() {
    for (index, kind) in [
        EffectKind::HighPass,
        EffectKind::EqBand,
        EffectKind::Compressor,
        EffectKind::Limiter,
        EffectKind::Denoise,
        EffectKind::Pitch,
        EffectKind::CenterCut,
    ]
    .into_iter()
    .enumerate()
    {
        let refused = probe_strings()[index].clone();
        let filters = probe(|filter| filter != refused);
        assert_eq!(filters.missing(), [kind]);
        assert!(!filters.unknown());
        for other in EffectKind::ALL {
            assert_eq!(
                filters.has(other),
                other != kind,
                "{other:?} without {kind:?}"
            );
        }
    }
}

#[test]
fn a_failing_control_leaves_every_kind_available_and_says_so_once() {
    let mut runs = 0;
    let filters = probe(|_| {
        runs += 1;
        false
    });
    assert_eq!(runs, 1, "nothing is run after a control that fails");
    assert!(filters.unknown() && filters.missing().is_empty());
    assert!(EffectKind::ALL.into_iter().all(|kind| filters.has(kind)));
    // Every kind is written into a chain as if it had been proved.
    let clip = clip_with(vec![effect(EffectKind::Denoise, &[])]);
    assert_eq!(
        effects(&clip, 48_000, 2, &filters),
        "afftdn=nr=18.000:nf=-40.000"
    );

    let (sender, events) = mpsc::sync_channel(8);
    let woken = Arc::new(AtomicUsize::new(0));
    let wake: Wake = {
        let woken = Arc::clone(&woken);
        Arc::new(move || {
            woken.fetch_add(1, Ordering::AcqRel);
        })
    };
    tell(&filters, 0, &sender, &wake);
    tell(&filters, 0, &sender, &wake);
    tell(&filters, left_out(&clip, 2, &filters), &sender, &wake);
    let said: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
        .map(|event| match event {
            AudioEvent::Message(message) => message,
            AudioEvent::Output(error) => error.to_string(),
        })
        .collect();
    assert_eq!(
        said,
        ["Audio: FFmpeg's filters could not be checked; audio effects are taken as available"]
    );
    assert_eq!(woken.load(Ordering::Acquire), 1);
}

#[test]
fn a_kind_left_out_of_a_chain_is_said_once_a_session() {
    use EffectKind as K;
    let filters = Filters::without(&[K::Denoise, K::Limiter]);
    let clip = clip_with(vec![effect(K::Denoise, &[]), effect(K::HighPass, &[])]);
    let (sender, events) = mpsc::sync_channel(1);
    let wake: Wake = Arc::new(|| {});
    let said = || match events.try_recv() {
        Ok(AudioEvent::Message(message)) => Some(message),
        _ => None,
    };

    // The limiter is missing too, and no clip asked for it: nothing is said.
    tell(&filters, 0, &sender, &wake);
    assert_eq!(said(), None);
    tell(&filters, left_out(&clip, 2, &filters), &sender, &wake);
    assert_eq!(
        said().as_deref(),
        Some("Audio: noise reduction is not available in this FFmpeg")
    );
    // The next voice that goes without it, and the next mixer, say nothing.
    tell(&filters, left_out(&clip, 2, &filters), &sender, &wake);
    assert_eq!(said(), None);

    // A notice that finds no room is not lost: it is said the next time.
    let both = bit_of(K::Limiter) | bit_of(K::Denoise);
    sender
        .try_send(AudioEvent::Message("in the way".into()))
        .unwrap();
    tell(&filters, both, &sender, &wake);
    assert_eq!(said().as_deref(), Some("in the way"));
    tell(&filters, both, &sender, &wake);
    assert_eq!(
        said().as_deref(),
        Some("Audio: the limiter is not available in this FFmpeg")
    );
    tell(&filters, both, &sender, &wake);
    assert_eq!(said(), None);
}

#[test]
fn the_hosts_ffmpeg_refuses_an_unknown_option_and_the_probe_marks_that_kind() {
    let _heavy = crate::heavy_test();
    if !decoder_available("filter probe") {
        return;
    }
    let processes = Processes::default();
    assert!(runs(&processes, "anull"), "the control runs");
    assert!(!runs(&processes, "alimiter=limit=1:nosuchoption=1"));
    assert!(!runs(&processes, "nosuchfilter"));

    // The session's probe, with the limiter's string given an option no
    // build has: the limiter is missing, by running and not by a listing.
    let strings = probe_strings();
    let started = Instant::now();
    let filters = probe(|filter| {
        if filter == strings[3] {
            runs(&processes, &format!("{filter}:nosuchoption=1"))
        } else {
            runs(&processes, filter)
        }
    });
    println!(
        "MEASURE: the probe's eight runs took {} ms; missing on this host besides the limiter: {:?}",
        started.elapsed().as_millis(),
        (filters.missing().iter())
            .filter(|kind| **kind != EffectKind::Limiter)
            .collect::<Vec<_>>()
    );
    assert!(!filters.unknown());
    assert!(filters.missing().contains(&EffectKind::Limiter));
    assert_eq!(processes.started_tools(), 3 + 8);
}

fn no_device(
    _: &Arc<AudioState>,
    _: &SyncSender<AudioEvent>,
    _: &Wake,
) -> Result<Output, MediaError> {
    Err(MediaError::Decode(
        "No native audio output device is available".into(),
    ))
}

fn said_within(service: &AudioService, seconds: u64) -> Option<String> {
    let until = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(message) = service.poll() {
            return Some(message);
        }
        if Instant::now() >= until {
            return None;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_probe_runs_on_a_host_without_an_output_device() {
    let probed = Arc::new(AtomicBool::new(false));
    let ran = Arc::clone(&probed);
    let service = AudioService::start(
        Arc::new(|| {}),
        move || {
            ran.store(true, Ordering::Release);
            // A control that fails, as with no FFmpeg at all.
            Arc::new(probe(|_| false))
        },
        no_device,
    )
    .unwrap();
    // What the probe found is said before the device is found missing.
    assert_eq!(
        said_within(&service, 5).as_deref(),
        Some("Audio: FFmpeg's filters could not be checked; audio effects are taken as available")
    );
    assert!(probed.load(Ordering::Acquire));
    assert_eq!(
        said_within(&service, 5).as_deref(),
        Some("No native audio output device is available")
    );
    assert!(service.state.unavailable());
}

#[test]
fn a_sync_during_the_probe_does_not_cancel_it() {
    let _heavy = crate::heavy_test();
    if !decoder_available("probe cancellation") {
        return;
    }
    let (running, is_running) = mpsc::channel();
    let (synced, is_synced) = mpsc::channel::<()>();
    let (answer, answered) = mpsc::channel();
    let service = AudioService::start(
        Arc::new(|| {}),
        move || {
            // As `chain::session` has it: processes of the probe alone. One
            // of its children is held open while the service is synced.
            let processes = Processes::default();
            let mut command = tool_command("ffmpeg");
            command.args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-f",
                "f32le",
                "pipe:1",
            ]);
            let mut child = Process::spawn("ffmpeg", &mut command, &processes).unwrap();
            let mut first = [0; 16];
            child.stdout.read_exact(&mut first).unwrap();
            running.send(()).unwrap();
            is_synced.recv().unwrap();
            thread::sleep(Duration::from_millis(100));
            let alive = child.child.lock().unwrap().try_wait().unwrap().is_none();
            drop(child);
            // And a run started after the sync ends well too.
            let filters = probe(|filter| runs(&processes, filter));
            answer.send((alive, filters.unknown())).unwrap();
            Arc::new(filters)
        },
        no_device,
    )
    .unwrap();
    is_running.recv_timeout(Duration::from_secs(10)).unwrap();
    // A seek, a play and a stop: each kills every child of the service.
    let project = Arc::new(Project::new("Probe fixture"));
    service.sync(Arc::clone(&project), Time::ZERO, true, 1.0);
    service.sync(project, Time::from_seconds(1.0), false, 1.0);
    service.stop();
    synced.send(()).unwrap();
    let (alive, unknown) = answered.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(alive, "the probe's child outlived the syncs");
    assert!(!unknown, "the control ran after the syncs");
}

#[test]
fn an_equalizer_band_lowers_its_own_frequency_and_leaves_one_far_from_it() {
    let _heavy = crate::heavy_test();
    if !decoder_available("equalizer") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let cut = || {
        vec![effect(
            EffectKind::EqBand,
            &[(ParamKey::Frequency, 1_000.0), (ParamKey::Gain, -24.0)],
        )]
    };
    let mut changes = Vec::new();
    for hertz in [1_000.0, 100.0] {
        let written = tone(hertz, 0.5, 1.0);
        let asset = stereo(&temp.path().join("tone.wav"), &written, &written);
        let plain = level(&heard(&asset, vec![], 0.95), 0.3, 0.9);
        assert!((plain - level(&written, 0.3, 0.9)).abs() < 0.01);
        changes.push(level(&heard(&asset, cut(), 0.95), 0.3, 0.9) - plain);
    }
    println!(
        "MEASURE: equalizer -24 dB at 1 kHz, one octave: {:.2} dB at 1 kHz, {:.2} dB at 100 Hz",
        changes[0], changes[1]
    );
    assert!((-24.0..=-20.0).contains(&changes[0]), "{}", changes[0]);
    assert!(changes[1].abs() < 1.0, "{}", changes[1]);
}

#[test]
fn a_high_pass_lowers_a_tone_under_its_frequency() {
    let _heavy = crate::heavy_test();
    if !decoder_available("high-pass") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let pass = || {
        vec![effect(
            EffectKind::HighPass,
            &[(ParamKey::Frequency, 200.0)],
        )]
    };
    let mut changes = Vec::new();
    for hertz in [50.0, 1_000.0] {
        let written = tone(hertz, 0.5, 1.0);
        let asset = stereo(&temp.path().join("tone.wav"), &written, &written);
        let plain = level(&heard(&asset, vec![], 0.95), 0.3, 0.9);
        changes.push(level(&heard(&asset, pass(), 0.95), 0.3, 0.9) - plain);
    }
    println!(
        "MEASURE: high-pass at 200 Hz: {:.2} dB at 50 Hz, {:.2} dB at 1 kHz",
        changes[0], changes[1]
    );
    assert!(changes[0] < -18.0, "{}", changes[0]);
    assert!(changes[1].abs() < 0.5, "{}", changes[1]);
}

#[test]
fn a_limiter_holds_a_full_scale_tone_at_its_ceiling() {
    let _heavy = crate::heavy_test();
    if !decoder_available("limiter effect") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(997.0, 1.0, 1.0);
    let asset = stereo(&temp.path().join("tone.wav"), &written, &written);
    let limited = heard(
        &asset,
        vec![effect(EffectKind::Limiter, &[(ParamKey::Threshold, -12.0)])],
        0.95,
    );
    let ceiling = 10_f32.powf(-12.0 / 20.0);
    let peak = limited.iter().fold(0_f32, |peak, s| peak.max(s.abs()));
    println!(
        "MEASURE: limiter at -12 dB (ceiling {ceiling:.4}) over a full-scale tone: peak {peak:.4}, {:.2} dBFS RMS",
        level(&limited, 0.3, 0.9)
    );
    assert!(peak <= ceiling * 1.001, "peak {peak} over {ceiling}");
    // It is held there, not silenced: a sine at the ceiling is 3 dB under it.
    assert!(level(&limited, 0.3, 0.9) > -12.0 - 3.01 - 3.0);
    // The 5 ms it delays the sound by are dropped: the tone is there at once.
    assert!(limited[..240].iter().any(|sample| sample.abs() > 0.1));
}

#[test]
fn a_compressor_lowers_a_tone_over_its_threshold_and_not_one_under_it() {
    let _heavy = crate::heavy_test();
    if !decoder_available("compressor") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // The defaults: a threshold of -18 dB and a ratio of 3.
    let mut changes = Vec::new();
    for amplitude in [0.5, 0.01] {
        let written = tone(997.0, amplitude, 1.0);
        let asset = stereo(&temp.path().join("tone.wav"), &written, &written);
        let plain = level(&heard(&asset, vec![], 0.95), 0.4, 0.9);
        let pressed = heard(&asset, vec![effect(EffectKind::Compressor, &[])], 0.95);
        changes.push(level(&pressed, 0.4, 0.9) - plain);
    }
    println!(
        "MEASURE: compressor at its defaults (attack 10 ms, release 150 ms): {:.2} dB on a tone at -6 dBFS peak, {:.2} dB on one at -40",
        changes[0], changes[1]
    );
    assert!(changes[0] < -3.0, "{}", changes[0]);
    assert!(changes[1].abs() < 0.5, "{}", changes[1]);
}

#[test]
fn centre_reduction_silences_dual_mono_and_keeps_what_is_hard_panned() {
    let _heavy = crate::heavy_test();
    if !decoder_available("centre reduction") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(997.0, 0.5, 0.5);
    let silence = vec![0_f32; written.len()];
    let cut = |asset: &Asset| {
        let mut clip = Clip::from_asset(asset, Time::ZERO);
        clip.effects = vec![effect(EffectKind::CenterCut, &[])];
        let project = project_of(&[asset], vec![clip]);
        heard_at(&project, SOURCE_RATE, Filters::default(), 0.45)
    };

    let centred = stereo(&temp.path().join("centre.wav"), &written, &written);
    let (left, right, _) = cut(&centred);
    assert!(left.iter().chain(&right).all(|sample| *sample == 0.0));

    // Sound on the left alone stays on the left as it was written. The
    // filter takes each channel from the other, so it is on the right too,
    // turned over.
    let panned = stereo(&temp.path().join("left.wav"), &written, &silence);
    let (left, right, _) = cut(&panned);
    for (frame, (left, right)) in left.iter().zip(&right).enumerate() {
        assert_eq!(left.to_bits(), written[frame].to_bits(), "frame {frame}");
        assert_eq!(*right, -written[frame], "frame {frame}");
    }
}

#[test]
fn centre_reduction_is_left_out_of_a_chain_for_one_channel_and_said_once() {
    use EffectKind as K;
    let clip = clip_with(vec![effect(K::CenterCut, &[]), effect(K::HighPass, &[])]);
    let all = Filters::default();
    let cut = "aformat=channel_layouts=stereo,pan=stereo|c0=c0-1.0000*c1|c1=c1-1.0000*c0";
    for channels in [2, 6] {
        assert_eq!(
            effects(&clip, 48_000, channels, &all),
            format!("{cut},highpass=f=80.000")
        );
        assert_eq!(left_out(&clip, channels, &all), 0);
    }
    assert_eq!(effects(&clip, 48_000, 1, &all), "highpass=f=80.000");
    assert_eq!(
        chain(&clip, 1.0, 1.0, 48_000, 1, &all).unwrap(),
        format!("{PLAIN},highpass=f=80.000")
    );
    assert_eq!(left_out(&clip, 1, &all), ONE_CHANNEL_BIT);

    let (sender, events) = mpsc::sync_channel(2);
    let wake: Wake = Arc::new(|| {});
    let said = || match events.try_recv() {
        Ok(AudioEvent::Message(message)) => Some(message),
        _ => None,
    };
    tell(&all, left_out(&clip, 1, &all), &sender, &wake);
    assert_eq!(
        said().as_deref(),
        Some("Audio: centre reduction needs two output channels and is not heard on this device")
    );
    tell(&all, left_out(&clip, 1, &all), &sender, &wake);
    assert_eq!(said(), None);

    // An FFmpeg that does not run it says that, whatever the channels, and
    // a disabled one went without nothing.
    let without = Filters::without(&[K::CenterCut]);
    assert_eq!(left_out(&clip, 1, &without), bit_of(K::CenterCut));
    let mut disabled = clip.clone();
    disabled.effects[0].enabled = false;
    assert_eq!(left_out(&disabled, 1, &all), 0);
}

#[test]
fn one_output_channel_hears_a_clip_with_centre_reduction_as_it_is_without() {
    let _heavy = crate::heavy_test();
    if !decoder_available("centre reduction in one channel") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let asset = stereo(
        &temp.path().join("two-tones.wav"),
        &tone(997.0, 0.5, 0.5),
        &tone(440.0, 0.25, 0.5),
    );
    let through = |effects: Vec<Effect>| {
        let mut clip = Clip::from_asset(&asset, Time::ZERO);
        clip.effects = effects;
        mixed(
            &project_of(&[&asset], vec![clip]),
            SOURCE_RATE,
            1,
            Filters::default(),
            0.45,
        )
    };

    // FFmpeg's sum of the filter's two channels would be silence here.
    let (plain, mixer) = through(Vec::new());
    assert_eq!(mixer.left_out, 0);
    assert!(level(&plain, 0.05, 0.4) > -20.0);
    let (cut, mixer) = through(vec![effect(EffectKind::CenterCut, &[])]);
    assert_eq!(mixer.left_out, ONE_CHANNEL_BIT);
    assert_eq!(cut.len(), plain.len());
    for (frame, (cut, plain)) in cut.iter().zip(&plain).enumerate() {
        assert_eq!(cut.to_bits(), plain.to_bits(), "frame {frame}");
    }
}

/// White noise of a known level from a fixed seed: the sum of four uniform
/// values, scaled to `rms`.
fn noise(frames: usize, rms: f64) -> Vec<f64> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut uniform = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1_u64 << 53) as f64 - 0.5
    };
    // A uniform value on -0.5..0.5 has variance 1/12; four of them 1/3.
    let scale = rms * 3_f64.sqrt();
    (0..frames)
        .map(|_| scale * (uniform() + uniform() + uniform() + uniform()))
        .collect()
}

#[test]
fn noise_reduction_lowers_the_noise_between_bursts_and_keeps_the_bursts() {
    let _heavy = crate::heavy_test();
    if !decoder_available("noise reduction") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // 300 Hz and 1 kHz for half a second in each second, over white noise
    // at -45 dBFS RMS.
    let rate = f64::from(SOURCE_RATE);
    let written: Vec<f32> = noise(4 * SOURCE_RATE as usize, 10_f64.powf(-45.0 / 20.0))
        .into_iter()
        .enumerate()
        .map(|(frame, noise)| {
            let at = frame as f64 / rate;
            let tones = 0.25 * (TAU * 300.0 * at).sin() + 0.25 * (TAU * 1_000.0 * at).sin();
            (noise + if at.fract() < 0.5 { tones } else { 0.0 }) as f32
        })
        .collect();
    let asset = stereo(&temp.path().join("noisy.wav"), &written, &written);
    // The middle of each gap and of each burst, the first second left out
    // for the filter to settle.
    let gaps = |samples: &[f32]| {
        (1..=3)
            .map(|second| level(samples, f64::from(second) + 0.6, f64::from(second) + 0.9))
            .sum::<f64>()
            / 3.0
    };
    let bursts = |samples: &[f32]| {
        (1..=3)
            .map(|second| level(samples, f64::from(second) + 0.1, f64::from(second) + 0.4))
            .sum::<f64>()
            / 3.0
    };
    let plain = heard(&asset, vec![], 3.95);
    assert!((gaps(&plain) + 45.0).abs() < 0.3, "{}", gaps(&plain));

    let reduced = |params: &[(ParamKey, f32)]| {
        let samples = heard(&asset, vec![effect(EffectKind::Denoise, params)], 3.95);
        (
            gaps(&samples) - gaps(&plain),
            bursts(&samples) - bursts(&plain),
        )
    };
    // The defaults are also what the Voice preset writes: 0.4 and -40 dB.
    let (at_defaults, bursts_at_defaults) = reduced(&[]);
    let (at_fifth, _) = reduced(&[(ParamKey::Amount, 0.2)]);
    let (at_one, bursts_at_one) = reduced(&[(ParamKey::Amount, 1.0)]);
    let (under_the_noise, _) = reduced(&[(ParamKey::Threshold, -80.0)]);
    println!(
        "MEASURE: noise at -45 dBFS between bursts: {at_defaults:.1} dB at the defaults (nr 18, nf -40), \
         {at_fifth:.1} dB at amount 0.2, {at_one:.1} dB at amount 1, {under_the_noise:.1} dB with the \
         floor at -80; bursts {bursts_at_defaults:.2} dB at the defaults, {bursts_at_one:.2} dB at amount 1"
    );
    assert!(at_defaults <= -10.0, "{at_defaults}");
    assert!(at_one < at_fifth, "{at_one} against {at_fifth}");
    assert!(bursts_at_defaults.abs() < 1.0, "{bursts_at_defaults}");
    assert!(bursts_at_one.abs() < 1.0, "{bursts_at_one}");
    // With the floor under the real noise it does next to nothing.
    assert!(under_the_noise.abs() < 3.0, "{under_the_noise}");
}

/// The first frame a 5 ms burst is heard at.
fn onset(samples: &[f32]) -> usize {
    (samples.iter())
        .position(|sample| sample.abs() > 0.05)
        .expect("the burst is heard")
}

#[test]
fn the_delay_of_a_chain_is_taken_off_the_start_of_its_voice_at_two_rates() {
    let _heavy = crate::heavy_test();
    use EffectKind as K;
    if !decoder_available("chain delay") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // 5 ms of 1 kHz at half scale, 300 ms into silence.
    let rate = f64::from(SOURCE_RATE);
    let written: Vec<f32> = (0..(0.6 * rate) as usize)
        .map(|frame| {
            let at = frame as f64 / rate;
            if (0.3..0.305).contains(&at) {
                (0.5 * (TAU * 1_000.0 * (at - 0.3)).sin()) as f32
            } else {
                0.0
            }
        })
        .collect();
    let asset = stereo(&temp.path().join("burst.wav"), &written, &written);
    let chains: [(&str, Vec<K>); 7] = [
        ("high-pass", vec![K::HighPass]),
        ("equalizer band", vec![K::EqBand]),
        ("compressor", vec![K::Compressor]),
        ("limiter", vec![K::Limiter]),
        ("noise reduction", vec![K::Denoise]),
        ("centre reduction at 0.5", vec![K::CenterCut]),
        ("noise reduction and limiter", vec![K::Denoise, K::Limiter]),
    ];
    for rate in [44_100_u32, 48_000] {
        let landed = |kinds: &[K]| {
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.effects = (kinds.iter())
                .map(|kind| match kind {
                    // At 1 it would silence this dual-mono burst.
                    K::CenterCut => effect(*kind, &[(ParamKey::Amount, 0.5)]),
                    kind => effect(*kind, &[]),
                })
                .collect();
            let dropped = latency_frames(&clip, rate, 2, &Filters::default());
            let project = project_of(&[&asset], vec![clip]);
            let (left, _, _) = heard_at(&project, rate, Filters::default(), 0.55);
            (onset(&left) as i64, dropped as i64)
        };
        let (place, _) = landed(&[]);
        let micros = |frames: i64| frames * 1_000_000 / i64::from(rate);
        assert!((micros(place) - 300_000).abs() < 200, "{}", micros(place));
        for (name, kinds) in &chains {
            let (at, dropped) = landed(kinds);
            // What the filters delayed the burst by is what was dropped and
            // what is left of it after.
            let left = micros(at - place);
            println!(
                "MEASURE: {rate} Hz, {name}: delayed {} us, {} us dropped ({dropped} frames), lands {left} us from its place",
                micros(at - place + dropped),
                micros(dropped),
            );
            assert!(
                left.abs() <= 1_000,
                "{name} at {rate} Hz lands {left} us off"
            );
        }
    }
}

#[test]
fn a_missing_kind_is_left_out_of_the_voice_and_its_delay_is_not_dropped() {
    let _heavy = crate::heavy_test();
    use EffectKind as K;
    if !decoder_available("missing kind") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(997.0, 0.5, 0.25);
    let asset = stereo(&temp.path().join("tone.wav"), &written, &written);
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.effects = vec![effect(K::Denoise, &[]), effect(K::Limiter, &[])];
    let project = project_of(&[&asset], vec![clip]);

    // Neither runs here: the voice is the plain one, sample for sample.
    let (left, right, mixer) = heard_at(
        &project,
        SOURCE_RATE,
        Filters::without(&[K::Denoise, K::Limiter]),
        0.2,
    );
    for (frame, (left, right)) in left.iter().zip(&right).enumerate() {
        assert_eq!(left.to_bits(), written[frame].to_bits(), "frame {frame}");
        assert_eq!(right.to_bits(), written[frame].to_bits(), "frame {frame}");
    }
    assert_eq!(mixer.left_out, bit_of(K::Denoise) | bit_of(K::Limiter));
    assert!(mixer.voices.values().all(|voice| voice.lead == 0));

    // With both, the delay is dropped once, when the voice starts.
    let (left, _, mixer) = heard_at(&project, SOURCE_RATE, Filters::default(), 0.2);
    assert_eq!(mixer.left_out, 0);
    assert_eq!(mixer.decoder_audit.opened, 1);
    assert!(mixer.voices.values().all(|voice| voice.lead == 0));
    assert!(left[..48].iter().any(|sample| sample.abs() > 0.1));
}

// ---- Pitch and pitch-keeping speed ----------------------------------------

fn pitched(semitones: f32) -> Effect {
    effect(EffectKind::Pitch, &[(ParamKey::Amount, semitones)])
}

/// The speed stage of a chain: what `asetrate` is given and the `atempo`
/// factors, as written.
fn stage_of(chain: &str) -> (u32, Vec<f64>) {
    let mut parts = chain.split(',');
    let rate = (parts.nth(1))
        .and_then(|part| part.strip_prefix("asetrate="))
        .and_then(|rate| rate.parse().ok())
        .unwrap_or_else(|| panic!("{chain}"));
    let tempos = (chain.split(','))
        .filter_map(|part| part.strip_prefix("atempo="))
        .map(|tempo| tempo.parse().unwrap())
        .collect();
    (rate, tempos)
}

#[test]
fn a_pitch_makes_one_tempo_stage_and_the_clip_still_plays_at_its_speed() {
    let all = Filters::default();
    for speed in [0.05, 0.5, 1.0, 2.0, 3.0, 16.0] {
        // No `Pitch`: today's string, the speed in the rate and no tempo.
        let plain = chain(&clip_with(vec![]), speed, 1.0, 48_000, 2, &all).unwrap();
        assert_eq!(
            plain,
            format!(
                "aresample=48000:async=1:first_pts=0,asetrate={},aresample=48000",
                (48_000.0 * speed).round()
            )
        );
        assert_eq!(speed_stage(speed, None), (speed, None));
        for semitones in [0.0, 12.0, -12.0] {
            let ratio = 2_f64.powf(f64::from(semitones) / 12.0);
            let (changes, tempo) = speed_stage(speed, Some(semitones));
            let tempo_or_one = tempo.unwrap_or(1.0);
            assert!((0.5..=2.0).contains(&tempo_or_one), "{speed} {semitones}");
            assert!(
                (changes * tempo_or_one - speed).abs() < 1e-6,
                "{speed}x at {semitones}: {changes} x {tempo_or_one}"
            );
            if (0.5..=2.0).contains(&(speed / ratio)) {
                assert!((changes - ratio).abs() < 1e-12, "{speed}x at {semitones}");
            }
            let string = chain(
                &clip_with(vec![pitched(semitones)]),
                speed,
                1.0,
                48_000,
                2,
                &all,
            )
            .unwrap();
            let (rate, tempos) = stage_of(&string);
            assert_eq!(f64::from(rate), (48_000.0 * changes).round(), "{string}");
            assert!(tempos.len() <= 1, "{string}");
            assert_eq!(tempos.first().copied(), tempo, "{string}");
            println!("MEASURE: {speed}x, pitch {semitones}: {string}");
        }
    }

    // The strings themselves. Speed 2 keeping pitch; an octave up at the
    // same length; speed 4 keeping pitch is past one stage, so an octave up
    // and not two; 16x and 0.05x the same beyond each end.
    for (speed, semitones, expected) in [
        (2.0, 0.0, "asetrate=48000,aresample=48000,atempo=2.000000"),
        (1.0, 12.0, "asetrate=96000,aresample=48000,atempo=0.500000"),
        (1.0, -12.0, "asetrate=24000,aresample=48000,atempo=2.000000"),
        (4.0, 0.0, "asetrate=96000,aresample=48000,atempo=2.000000"),
        (4.0, 12.0, "asetrate=96000,aresample=48000,atempo=2.000000"),
        (16.0, 0.0, "asetrate=384000,aresample=48000,atempo=2.000000"),
        (0.05, 0.0, "asetrate=4800,aresample=48000,atempo=0.500000"),
        // A speed the pitch ratio takes out whole needs no tempo stage.
        (2.0, 12.0, "asetrate=96000,aresample=48000"),
        (1.0, 0.0, "asetrate=48000,aresample=48000"),
    ] {
        let string = chain(
            &clip_with(vec![pitched(semitones)]),
            speed,
            1.0,
            48_000,
            2,
            &all,
        )
        .unwrap();
        assert_eq!(
            string,
            format!("aresample=48000:async=1:first_pts=0,{expected}"),
            "{speed}x at {semitones}"
        );
    }

    // An `atempo` within 0.001 of 1 is left out and the rate is the speed.
    assert_eq!(speed_stage(1.0005, Some(0.0)), (1.0005, None));
    assert_eq!(speed_stage(1.002, Some(0.0)), (1.0, Some(1.002)));
}

#[test]
fn only_the_first_enabled_pitch_this_ffmpeg_runs_is_the_speed_stage() {
    use EffectKind as K;
    let all = Filters::default();
    let mut off = pitched(-12.0);
    off.enabled = false;
    // A disabled one is no switch; the first enabled one is read.
    let disabled = clip_with(vec![off.clone()]);
    assert_eq!(
        chain(&disabled, 2.0, 1.0, 48_000, 2, &all).unwrap(),
        "aresample=48000:async=1:first_pts=0,asetrate=96000,aresample=48000"
    );
    let two = clip_with(vec![off, pitched(12.0), pitched(0.0)]);
    assert_eq!(
        chain(&two, 2.0, 1.0, 48_000, 2, &all).unwrap(),
        "aresample=48000:async=1:first_pts=0,asetrate=96000,aresample=48000"
    );
    // No `atempo` delay is taken off the voice.
    assert_eq!(
        latency_frames(&clip_with(vec![pitched(0.0)]), 48_000, 2, &all),
        0
    );

    // An FFmpeg that cannot run it: speed changes pitch as it always did,
    // and that is said.
    let keep = clip_with(vec![pitched(0.0)]);
    let without = Filters::without(&[K::Pitch]);
    assert_eq!(
        chain(&keep, 2.0, 1.0, 48_000, 2, &without).unwrap(),
        "aresample=48000:async=1:first_pts=0,asetrate=96000,aresample=48000"
    );
    assert_eq!(left_out(&keep, 2, &without), bit_of(K::Pitch));
    assert_eq!(
        without.unsaid(left_out(&keep, 2, &without))[0].1,
        "Audio: pitch is not available in this FFmpeg"
    );
}

#[test]
fn the_transport_still_shifts_pitch_and_the_resampling_bound_is_on_the_rate() {
    let all = Filters::default();
    let keep = clip_with(vec![pitched(0.0)]);
    // At half the transport's speed a clip at 2x keeping pitch is an octave
    // down, at the same tempo stage.
    assert_eq!(
        chain(&keep, 2.0, 0.5, 48_000, 2, &all).unwrap(),
        "aresample=48000:async=1:first_pts=0,asetrate=24000,aresample=48000,atempo=2.000000"
    );
    // 16x at a 4x transport: 3,072,000 Hz without the tempo stage, past the
    // bound; with it the rate is 48000 x 4 x 8, at the bound.
    let refused = chain(&clip_with(vec![]), 16.0, 4.0, 48_000, 2, &all).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Clip and preview speeds exceed the audio resampling range"
    );
    assert_eq!(
        chain(&keep, 16.0, 4.0, 48_000, 2, &all).unwrap(),
        "aresample=48000:async=1:first_pts=0,asetrate=1536000,aresample=48000,atempo=2.000000"
    );
    // 0.05x at a 0.03x transport: 72 Hz plain, 144 Hz an octave up and
    // stretched.
    assert!(chain(&clip_with(vec![]), 0.05, 0.03, 48_000, 2, &all).is_err());
    assert_eq!(
        chain(&clip_with(vec![pitched(12.0)]), 0.05, 0.03, 48_000, 2, &all).unwrap(),
        "aresample=48000:async=1:first_pts=0,asetrate=144,aresample=48000,atempo=0.500000"
    );
}

/// The frequency of a tone from its zero crossings between `from` and `to`
/// seconds at 48 kHz.
fn hertz(samples: &[f32], from: f64, to: f64) -> f64 {
    let at = |seconds: f64| (seconds * f64::from(SOURCE_RATE)) as usize;
    let crossings = (samples[at(from)..at(to)].windows(2))
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    crossings as f64 / 2.0 / (to - from)
}

/// The time, in seconds, of the last sample above a tenth of full scale.
fn last_heard(samples: &[f32]) -> f64 {
    let last = (samples.iter())
        .rposition(|sample| sample.abs() > 0.1)
        .expect("the tone is heard");
    (last + 1) as f64 / f64::from(SOURCE_RATE)
}

/// A clip of all of `asset` at `speed`, with these effects.
fn at_speed(asset: &Asset, speed: f64, effects: Vec<Effect>) -> Clip {
    let mut clip = Clip::from_asset(asset, Time::ZERO);
    clip.playback_rate = speed;
    clip.duration = Time((asset.duration.0 as f64 / speed).floor() as i64);
    clip.effects = effects;
    clip
}

/// What a tone is heard as: what is played, at what clip speed and with
/// what effects, its frequency, where it ends in seconds and how early the
/// end may be.
type ToneCase = (&'static str, f64, Vec<Effect>, f64, f64, f64);

#[test]
fn a_tone_keeps_its_pitch_at_twice_the_speed_and_is_shifted_by_a_pitch_alone() {
    let _heavy = crate::heavy_test();
    if !decoder_available("pitch") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // 440 Hz for 0.8 s, then 0.2 s of silence: where the tone ends says how
    // long it lasts.
    let mut written = tone(440.0, 0.5, 0.8);
    written.resize(SOURCE_RATE as usize, 0.0);
    let asset = stereo(&temp.path().join("a440.wav"), &written, &written);
    let read = |speed: f64, effects: Vec<Effect>| {
        let clip = at_speed(&asset, speed, effects);
        let (left, _, _) = heard_at(
            &project_of(&[&asset], vec![clip]),
            SOURCE_RATE,
            Filters::default(),
            1.0,
        );
        left
    };

    // `atempo` leads by up to 25 ms at 2 and 70 ms at 0.5.
    let cases: [ToneCase; 4] = [
        ("speed 1", 1.0, vec![], 440.0, 0.8, 0.001),
        (
            "speed 2 keeping pitch",
            2.0,
            vec![pitched(0.0)],
            440.0,
            0.4,
            0.025,
        ),
        (
            "an octave up at speed 1",
            1.0,
            vec![pitched(12.0)],
            880.0,
            0.8,
            0.070,
        ),
        ("speed 2 without a pitch", 2.0, vec![], 880.0, 0.4, 0.001),
    ];
    for (what, speed, effects, expected, end, early) in cases {
        let left = read(speed, effects);
        let measured = hertz(&left, 0.1, end - 0.1);
        let ended = last_heard(&left);
        println!(
            "MEASURE: {what}: {measured:.1} Hz (wanted {expected}), tone ends at {:.1} ms (wanted {:.1})",
            ended * 1_000.0,
            end * 1_000.0
        );
        assert!(
            (measured - expected).abs() <= expected * 0.01,
            "{what}: {measured} Hz"
        );
        assert!(
            ended >= end - early && ended <= end + 0.002,
            "{what}: ends at {ended} s"
        );
    }
}

#[test]
fn a_tone_burst_through_one_tempo_stage_arrives_early_within_its_bound() {
    let _heavy = crate::heavy_test();
    if !decoder_available("tempo onset") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // 5 ms of 1 kHz at half scale at 1, 10 and 30 s of a 31 s source.
    let rate = f64::from(SOURCE_RATE);
    let bursts = [1.0, 10.0, 30.0];
    let written: Vec<f32> = (0..(31.0 * rate) as usize)
        .map(|frame| {
            let at = frame as f64 / rate;
            match bursts
                .iter()
                .find(|burst| (**burst..*burst + 0.005).contains(&at))
            {
                Some(burst) => (0.5 * (TAU * 1_000.0 * (at - burst)).sin()) as f32,
                None => 0.0,
            }
        })
        .collect();
    let asset = stereo(&temp.path().join("bursts.wav"), &written, &written);
    // The tempo factor, which is the clip's speed with a `Pitch` at 0, and
    // the most an onset may be early by, in milliseconds.
    for (tempo, most_early) in [(0.5, 70.0), (0.84, 35.0), (1.5, 25.0), (2.0, 25.0)] {
        let clip = at_speed(&asset, tempo, vec![pitched(0.0)]);
        let seconds = clip.duration.as_seconds();
        let (left, _, _) = heard_at(
            &project_of(&[&asset], vec![clip]),
            SOURCE_RATE,
            Filters::default(),
            seconds - 0.1,
        );
        let mut early = Vec::new();
        for burst in bursts {
            let place = burst / tempo * rate;
            let from = (place - 0.2 * rate) as usize;
            let onset = from
                + (left[from..(place + 0.05 * rate) as usize].iter())
                    .position(|sample| sample.abs() > 0.05)
                    .unwrap_or_else(|| panic!("the burst at {burst} s is heard at {tempo}"));
            early.push((place - onset as f64) / rate * 1_000.0);
        }
        println!(
            "MEASURE: atempo={tempo}: onsets early by {:.1}, {:.1} and {:.1} ms at 1, 10 and 30 s of source",
            early[0], early[1], early[2]
        );
        for (burst, early) in bursts.iter().zip(&early) {
            assert!(
                (-2.0..=most_early).contains(early),
                "atempo={tempo}: the burst at {burst} s is {early:.1} ms early"
            );
        }
    }
}
