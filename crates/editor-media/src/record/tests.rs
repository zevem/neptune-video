//! Takes written through the callback's own function by a fake source that
//! the test drives, with the writer stalled, ended as a crash would end it
//! and failed at will; the files read back byte for byte and, where FFmpeg
//! is on the host, measured by ffprobe.
use super::*;
use crate::process::{Processes, capture, tool_command};
use std::{process::Stdio, sync::atomic::AtomicUsize};

/// What a test keeps of its fake source: the feed it was built with, and
/// whether it was played and dropped. The test's own thread is the device's.
#[derive(Clone, Default)]
struct Fake {
    feed: Arc<Mutex<Option<Arc<Feed>>>>,
    playing: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    /// Set to refuse `play`, as a busy device does after it was built.
    refuses: Arc<AtomicBool>,
    /// Frames of the ramp handed over as soon as it plays, before the file.
    early: Arc<AtomicU64>,
}

struct FakeSource(Fake);

impl Source for FakeSource {
    fn play(&self) -> Result<(), MediaError> {
        if self.0.refuses.load(Ordering::Acquire) {
            return Err(MediaError::Decode(
                "Cannot open the microphone: the device is in use".into(),
            ));
        }
        self.0.playing.store(true, Ordering::Release);
        let early = self.0.early.load(Ordering::Acquire);
        if early > 0 {
            feed_ramp(&self.0.feed(), 0, early, 80);
        }
        Ok(())
    }
}

impl Drop for FakeSource {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::Release);
    }
}

impl Fake {
    fn open(&self, rate: u32, channels: u16) -> Open {
        let fake = self.clone();
        Box::new(move || {
            Ok(Input {
                rate,
                channels,
                build: Box::new(move |feed| {
                    *fake.feed.lock().unwrap() = Some(feed);
                    Ok(Box::new(FakeSource(fake)) as Box<dyn Source>)
                }),
            })
        })
    }

    fn feed(&self) -> Arc<Feed> {
        self.feed
            .lock()
            .unwrap()
            .clone()
            .expect("the source was built")
    }

    fn dropped(&self) -> bool {
        self.dropped.load(Ordering::Acquire)
    }
}

/// A take at `path` from a fake source, with the wakes it asks for counted.
fn start(
    fake: &Fake,
    path: &Path,
    rate: u32,
    channels: u16,
    longest: Duration,
) -> (RecordService, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&wakes);
    let service = RecordService::start_with(
        path.to_path_buf(),
        Arc::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
        }),
        fake.open(rate, channels),
        longest,
    )
    .unwrap();
    (service, wakes)
}

fn next(service: &RecordService) -> RecordEvent {
    let limit = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = service.poll() {
            return event;
        }
        assert!(Instant::now() < limit, "the service said nothing");
        thread::sleep(Duration::from_millis(2));
    }
}

fn await_started(service: &RecordService) -> (u32, u16) {
    match next(service) {
        RecordEvent::Started { rate, channels } => (rate, channels),
        other => panic!("not started: {other:?}"),
    }
}

/// The take's last word, and the loudest level said before it.
fn await_end(service: &RecordService) -> (RecordEvent, f32) {
    let mut loudest = 0_f32;
    loop {
        match next(service) {
            RecordEvent::Level(level) => loudest = loudest.max(level),
            RecordEvent::Started { .. } => panic!("started twice"),
            end => return (end, loudest),
        }
    }
}

fn await_take(service: &RecordService) -> (Take, f32) {
    match await_end(service) {
        (RecordEvent::Finished(take), loudest) => (take, loudest),
        (other, _) => panic!("not finished: {other:?}"),
    }
}

fn await_until(what: &str, done: impl Fn() -> bool) {
    let limit = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < limit, "{what}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Sample `index` of a ramp: each one a different float, exactly as written.
fn ramp(index: u64) -> f32 {
    index as f32 / (1 << 24) as f32
}

/// Hands `frames` of the ramp, from frame `from`, to the callback in calls
/// of `chunk` frames, and returns the longest call.
fn feed_ramp(feed: &Feed, from: u64, frames: u64, chunk: u64) -> Duration {
    let channels = feed.channels as u64;
    let mut longest = Duration::ZERO;
    let mut at = from;
    while at < from + frames {
        let count = chunk.min(from + frames - at);
        let samples: Vec<f32> = (at * channels..(at + count) * channels).map(ramp).collect();
        let called = Instant::now();
        feed.capture(&samples);
        longest = longest.max(called.elapsed());
        at += count;
    }
    longest
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |metadata| metadata.len())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

/// The file's header and every sample after it.
fn read_wav(path: &Path) -> (Vec<u8>, Vec<f32>) {
    let bytes = fs::read(path).unwrap();
    let samples = bytes[44..]
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
        .collect();
    (bytes[..44].to_vec(), samples)
}

fn assert_header(header: &[u8], rate: u32, channels: u16, data: u32) {
    assert_eq!(&header[0..4], b"RIFF");
    assert_eq!(u32_at(header, 4), 36 + data);
    assert_eq!(&header[8..16], b"WAVEfmt ");
    assert_eq!(u32_at(header, 16), 16);
    assert_eq!(u16_at(header, 20), 3, "IEEE float");
    assert_eq!(u16_at(header, 22), channels);
    assert_eq!(u32_at(header, 24), rate);
    assert_eq!(u32_at(header, 28), rate * u32::from(channels) * 4);
    assert_eq!(u16_at(header, 32), channels * 4);
    assert_eq!(u16_at(header, 34), 32);
    assert_eq!(&header[36..40], b"data");
    assert_eq!(u32_at(header, 40), data);
}

/// The file's length as ffprobe reads it, or `None` without ffprobe.
fn probed_seconds(path: &Path) -> Option<f64> {
    let available = tool_command("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !available {
        eprintln!(
            "SKIP: ffprobe is unavailable; {} was not probed",
            path.display()
        );
        return None;
    }
    let mut command = tool_command("ffprobe");
    command
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "csv=p=0"])
        .arg(path);
    let bytes = capture("ffprobe", &mut command, &Processes::default(), 1 << 10).unwrap();
    Some(String::from_utf8(bytes).unwrap().trim().parse().unwrap())
}

const LONGEST: Duration = Duration::from_secs(MAX_TAKE_SECONDS);

// ---- What is written ---------------------------------------------------------------

#[test]
fn a_ramp_comes_back_sample_for_sample_under_a_header_whose_sizes_match() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, wakes) = start(&fake, &path, 48_000, 2, LONGEST);
    assert_eq!(service.frames(), 0);
    assert_eq!(await_started(&service), (48_000, 2));
    assert!(fake.playing.load(Ordering::Acquire));
    assert_eq!(service.rate(), 48_000);
    let feed = fake.feed();
    for second in 0..2 {
        for chunk in 0..100 {
            let at = second * 48_000 + chunk * 480;
            feed_ramp(&feed, at, 480, 480);
            assert_eq!(service.frames(), at + 480);
            // The file never holds more than the clock has counted.
            assert!(file_len(&path) <= 44 + service.frames() * 8);
        }
        // Each frame counted is the next in the file once it is drained.
        let counted = service.frames();
        await_until("the file did not catch up", || {
            file_len(&path) == 44 + counted * 8
        });
    }
    assert_eq!(service.elapsed(), Time(2_000_000));
    service.stop();
    let (take, loudest) = await_take(&service);
    assert_eq!(service.frames(), 96_000);
    assert_eq!(
        take,
        Take {
            path: fs::canonicalize(&path).unwrap(),
            frames: 96_000,
            rate: 48_000,
            channels: 2,
            bytes: 44 + 96_000 * 8,
            dropped: 0,
            ended: None,
        }
    );
    assert_eq!(take.bytes, file_len(&path));
    assert!(loudest > 0.0 && loudest <= ramp(96_000 * 2 - 1));
    assert!(
        wakes.load(Ordering::Relaxed) >= 2,
        "started and finished wake"
    );
    let (header, samples) = read_wav(&path);
    assert_header(&header, 48_000, 2, 96_000 * 8);
    assert_eq!(samples.len(), 96_000 * 2);
    for (index, sample) in samples.iter().enumerate() {
        assert_eq!(
            sample.to_bits(),
            ramp(index as u64).to_bits(),
            "sample {index}"
        );
    }
    if let Some(seconds) = probed_seconds(&path) {
        let expected = take.frames as f64 / f64::from(take.rate);
        eprintln!("MEASURE: ffprobe read {seconds} s of a {expected} s take");
        assert!(
            (seconds - expected).abs() <= 1.0 / 48_000.0,
            "ffprobe read {seconds} s for {expected} s"
        );
    }
}

#[test]
fn a_stalled_writer_counts_what_is_dropped_and_never_holds_up_the_callback() {
    let _heavy = crate::heavy_test();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    let feed = fake.feed();
    let held = service.shared.hold.lock().unwrap();
    // Three seconds into a ring of two.
    let longest_call = feed_ramp(&feed, 0, 24_000, 80);
    eprintln!(
        "MEASURE: longest callback of 80 mono frames, 300 calls into a stalled writer: {longest_call:?}"
    );
    assert!(
        longest_call < Duration::from_millis(5),
        "a callback took {longest_call:?}"
    );
    assert_eq!(service.frames(), 16_000, "only what reached the ring");
    assert_eq!(feed.dropped.load(Ordering::Acquire), 8_000);
    drop(held);
    service.stop();
    let (take, _) = await_take(&service);
    assert_eq!((take.frames, take.dropped), (16_000, 8_000));
    assert_eq!(take.bytes, file_len(&path));
    let (header, samples) = read_wav(&path);
    assert_header(&header, 8_000, 1, 16_000 * 4);
    assert!(
        samples
            .iter()
            .enumerate()
            .all(|(index, sample)| *sample == ramp(index as u64)),
        "the first two seconds, as they came"
    );
}

#[test]
fn stop_returns_at_once_while_the_writer_is_stalled() {
    let _heavy = crate::heavy_test();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    let feed = fake.feed();
    feed_ramp(&feed, 0, 4_000, 80);
    let held = service.shared.hold.lock().unwrap();
    let asked = Instant::now();
    service.stop();
    let took = asked.elapsed();
    eprintln!("MEASURE: stop with the writer stalled: {took:?}");
    assert!(took < Duration::from_millis(20), "stop took {took:?}");
    // Nothing that comes after the stop is taken.
    feed_ramp(&feed, 4_000, 800, 80);
    assert_eq!(service.frames(), 4_000);
    thread::sleep(Duration::from_millis(60));
    while let Some(event) = service.poll() {
        assert!(
            matches!(event, RecordEvent::Level(_)),
            "the stalled writer has not finished: {event:?}"
        );
    }
    drop(held);
    let (take, _) = await_take(&service);
    assert_eq!((take.frames, take.dropped), (4_000, 0));
}

#[test]
fn a_writer_ended_mid_take_leaves_a_file_that_plays_to_within_a_second() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    let feed = fake.feed();
    // Under the ring's two seconds at a time, each drained before the next.
    for quarter in 0..4 {
        feed_ramp(&feed, quarter * 5_000, 5_000, 80);
        let counted = service.frames();
        await_until("the writer did not drain", || {
            file_len(&path) == 44 + counted * 4
        });
    }
    assert_eq!(service.frames(), 20_000);
    // As a crash would: nothing is closed and no size is rewritten.
    service.shared.crash.store(true, Ordering::Release);
    drop(service);
    assert!(fake.dropped());
    let (header, _) = read_wav(&path);
    let declared = u64::from(u32_at(&header, 40)) / 4;
    eprintln!("MEASURE: header after an ended writer: {declared} of 20,000 frames");
    assert_eq!(u32_at(&header, 4), 36 + u32_at(&header, 40));
    assert!(
        declared <= 20_000 && 20_000 - declared <= 8_000,
        "the header counts {declared} of 20,000 frames"
    );
    if let Some(seconds) = probed_seconds(&path) {
        eprintln!("MEASURE: ffprobe after an ended writer: {seconds} s of 2.5 s");
        assert!(
            (1.5..=2.5).contains(&seconds),
            "ffprobe read {seconds} s of a 2.5 s take"
        );
    }
}

#[test]
fn the_take_ends_itself_at_its_longest_and_finishes_as_if_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 8_000, 1, Duration::from_millis(250));
    await_started(&service);
    let feed = fake.feed();
    feed_ramp(&feed, 0, 4_000, 80);
    assert_eq!(service.frames(), 2_000);
    let (take, _) = await_take(&service);
    assert_eq!(
        (take.frames, take.dropped),
        (2_000, 0),
        "past the end is not dropped"
    );
    assert_eq!(
        take.ended.as_deref(),
        Some("The take ended at its limit of 0.25 s")
    );
    assert_eq!(take.bytes, file_len(&path));
    assert!(fake.dropped());
}

#[test]
fn the_wav_bound_is_held_without_wrapping_either_size() {
    assert_eq!(
        limit(48_000, 1, LONGEST),
        (345_600_000, "The take ended at its limit of 2 hours".into())
    );
    // Stereo at 192 kHz meets the header's bound first: 4,294,967,256 bytes.
    let (most, said) = limit(192_000, 2, LONGEST);
    assert_eq!(most, 536_870_907);
    assert_eq!(said, "The take ended at the most a WAV file can hold");
    assert_eq!(
        limit(48_000, 1, Duration::from_secs(100 * 3600)).0,
        1_073_741_814
    );
    let bound = 536_870_907 * 8;
    assert_eq!(bound, 4_294_967_256_u64);
    // A writer stopped at the bound, given its length rather than 4 GiB of
    // samples.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bound.wav");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let mut wav = Wav::create(file, 192_000, 2).unwrap();
    wav.data = bound;
    wav.finish().unwrap();
    drop(wav);
    let written = &fs::read(&path).unwrap()[..44];
    assert_header(written, 192_000, 2, bound as u32);
    assert_eq!(u32_at(written, 4), 4_294_967_292);
    // Past it, the sizes are held, not wrapped.
    let past = header(192_000, 2, bound + 64);
    assert_eq!(u32_at(&past, 40), bound as u32);
}

// ---- Refusals and failures -----------------------------------------------------------

#[test]
fn a_file_already_there_is_refused_and_left_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    fs::write(&path, b"the person's").unwrap();
    let fake = Fake::default();
    let (service, wakes) = start(&fake, &path, 48_000, 2, LONGEST);
    match await_end(&service).0 {
        RecordEvent::Failed(MediaError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
        }
        other => panic!("not refused: {other:?}"),
    }
    assert_eq!(fs::read(&path).unwrap(), b"the person's");
    // Played before the file is made, and closed when it could not be.
    assert!(fake.playing.load(Ordering::Acquire) && fake.dropped());
    assert_eq!((service.frames(), service.rate()), (0, 0));
    assert!(wakes.load(Ordering::Relaxed) >= 1);
}

#[test]
fn a_device_that_will_not_play_fails_before_started_and_makes_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    fake.refuses.store(true, Ordering::Release);
    let (service, _) = start(&fake, &path, 48_000, 2, LONGEST);
    // `await_end` would panic on a `Started`.
    match await_end(&service).0 {
        RecordEvent::Failed(error) => assert_eq!(
            error.to_string(),
            "Cannot open the microphone: the device is in use"
        ),
        other => panic!("not refused: {other:?}"),
    }
    assert!(!path.exists());
    assert!(fake.dropped());
    assert_eq!((service.frames(), service.rate()), (0, 0));
    assert!(service.poll().is_none());
}

#[test]
fn frames_count_from_the_device_start_so_may_be_past_zero_at_started() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    fake.early.store(800, Ordering::Release);
    let (service, _) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    // What came while the file was made is counted, in the file at its place.
    assert_eq!(service.frames(), 800);
    feed_ramp(&fake.feed(), 800, 800, 80);
    service.stop();
    let (take, _) = await_take(&service);
    assert_eq!((take.frames, take.dropped), (1_600, 0));
    let (_, samples) = read_wav(&path);
    assert!(
        samples
            .iter()
            .enumerate()
            .all(|(index, sample)| *sample == ramp(index as u64))
    );
    assert_eq!(samples.len(), 1_600);
}

#[test]
fn a_full_disk_ends_the_take_keeps_what_was_written_and_cuts_the_part_piece() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, wakes) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    let feed = fake.feed();
    feed_ramp(&feed, 0, 4_000, 80);
    await_until("the writer did not drain", || {
        file_len(&path) == 44 + 4_000 * 4
    });
    service.shared.full.store(true, Ordering::Release);
    feed_ramp(&feed, 4_000, 800, 80);
    let (take, _) = await_take(&service);
    assert_eq!(
        take.ended.as_deref(),
        Some("Cannot write the take: No space left on device")
    );
    assert_eq!((take.frames, take.bytes), (4_000, 44 + 4_000 * 4));
    // Half a piece reached the disk before the write failed; it is cut off.
    assert_eq!(take.bytes, file_len(&path));
    let (header, samples) = read_wav(&path);
    assert_header(&header, 8_000, 1, 4_000 * 4);
    assert!(
        samples
            .iter()
            .enumerate()
            .all(|(index, sample)| *sample == ramp(index as u64)),
        "what was written before the disk filled"
    );
    assert!(fake.dropped());
    assert!(wakes.load(Ordering::Relaxed) >= 2);
}

#[test]
fn a_failed_write_after_a_device_error_keeps_both_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    let feed = fake.feed();
    // Held so that the writer sees the device's error before it writes.
    let held = service.shared.hold.lock().unwrap();
    feed_ramp(&feed, 0, 800, 80);
    service.shared.full.store(true, Ordering::Release);
    feed.fail("The microphone stopped: it was unplugged".into());
    drop(held);
    let (take, _) = await_take(&service);
    assert_eq!(
        take.ended.as_deref(),
        Some(
            "The microphone stopped: it was unplugged. \
             Cannot write the take: No space left on device"
        )
    );
    assert_eq!((take.frames, take.bytes), (0, 44));
    assert_eq!(take.bytes, file_len(&path));
}

#[test]
fn no_microphone_is_said_in_words_and_makes_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let service = RecordService::start_with(
        path.clone(),
        Arc::new(|| {}),
        Box::new(|| Err(MediaError::NoMicrophone)),
        LONGEST,
    )
    .unwrap();
    match await_end(&service).0 {
        RecordEvent::Failed(error @ MediaError::NoMicrophone) => {
            assert_eq!(error.to_string(), "No microphone");
        }
        other => panic!("not refused: {other:?}"),
    }
    assert!(!path.exists());
    assert_eq!((service.frames(), service.rate()), (0, 0));
    assert_eq!(service.elapsed(), Time::ZERO);
}

#[test]
fn more_than_two_channels_are_refused_before_the_source_is_built() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 48_000, 6, LONGEST);
    match await_end(&service).0 {
        RecordEvent::Failed(error) => assert_eq!(
            error.to_string(),
            "The microphone delivers 6 channels; only one or two can be recorded"
        ),
        other => panic!("not refused: {other:?}"),
    }
    assert!(fake.feed.lock().unwrap().is_none());
    assert!(!path.exists());
}

#[test]
fn a_stream_error_ends_the_take_and_finished_still_carries_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &path, 8_000, 1, LONGEST);
    await_started(&service);
    let feed = fake.feed();
    for _ in 0..50 {
        feed.capture(&[0.25_f32; 80]);
    }
    feed.fail("The microphone stopped: it was unplugged".into());
    let (take, loudest) = await_take(&service);
    assert_eq!(
        take.ended.as_deref(),
        Some("The microphone stopped: it was unplugged")
    );
    assert_eq!((take.frames, take.bytes), (4_000, file_len(&path)));
    assert_eq!(loudest, 0.25);
    let (header, samples) = read_wav(&path);
    assert_header(&header, 8_000, 1, 4_000 * 4);
    assert!(samples.iter().all(|sample| *sample == 0.25));
    assert!(fake.dropped());
}

#[test]
fn a_path_without_a_file_name_is_refused_before_any_thread() {
    let refused = RecordService::start_with(
        PathBuf::from("/"),
        Arc::new(|| {}),
        Box::new(|| panic!("no input is opened")),
        LONGEST,
    );
    assert!(matches!(refused, Err(MediaError::Invalid(_))));
}

// ---- The thread and the source ---------------------------------------------------------

#[test]
fn the_thread_ends_and_the_source_is_dropped_on_stop_and_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let stopped = dir.path().join("stopped.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &stopped, 8_000, 1, LONGEST);
    await_started(&service);
    feed_ramp(&fake.feed(), 0, 1_600, 80);
    service.stop();
    let (take, _) = await_take(&service);
    assert_eq!(take.frames, 1_600);
    assert!(fake.dropped());
    let thread = service.thread.as_ref().unwrap();
    await_until("the thread did not end on stop", || thread.is_finished());

    // Dropped without a stop: the take is closed whole before drop returns.
    let dropped = dir.path().join("dropped.wav");
    let fake = Fake::default();
    let (service, _) = start(&fake, &dropped, 8_000, 1, LONGEST);
    await_started(&service);
    feed_ramp(&fake.feed(), 0, 1_600, 80);
    drop(service);
    assert!(fake.dropped());
    let (header, samples) = read_wav(&dropped);
    assert_header(&header, 8_000, 1, 1_600 * 4);
    assert_eq!(samples.len(), 1_600);
}
