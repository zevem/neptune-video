//! The reduction on samples made here whose level is known, the cache on
//! files written here, the lane's queue with a job that waits to be told,
//! the loudness summary in reports FFmpeg wrote, beats in peaks made here
//! with clicks where they are known to be, and the whole of it on files
//! FFmpeg makes, and reads back once it has written them.
use super::{
    loudness::{graph, summary},
    peaks::{MAGIC, fnv},
    *,
};
use crate::audio::chain::Filters;
use crate::{import::asset_from_probe, process::capture};
use editor_core::{Effect, EffectKind, ParamKey, ParamValue};
use std::{fs, path::Path, process::Stdio, sync::atomic::AtomicUsize, time::SystemTime};

// ---- The reduction -------------------------------------------------------------

/// A sine at the lane's own rate, as the bytes FFmpeg would hand over.
fn sine(hertz: f64, amplitude: f64, samples: usize) -> Vec<u8> {
    (0..samples)
        .flat_map(|index| {
            let phase = std::f64::consts::TAU * hertz * index as f64 / f64::from(SAMPLE_RATE);
            ((amplitude * 32767.0 * phase.sin()).round() as i16).to_le_bytes()
        })
        .collect()
}

fn reduced(bytes: &[u8]) -> Peaks {
    let mut reducer = Reducer::default();
    reducer.push(bytes);
    reducer.finish()
}

/// The level byte of a sine of this amplitude: its level is 3.01 dB under
/// its peak.
fn sine_level(amplitude: f64) -> i32 {
    let decibels = 20.0 * (amplitude / std::f64::consts::SQRT_2).log10();
    ((decibels + 96.0) * 255.0 / 96.0).round() as i32
}

#[test]
fn a_tone_of_a_known_level_is_reduced_to_its_peaks_and_its_level() {
    // Half of full scale: -9.03 dBFS, byte 231, and 64 128ths either way.
    assert_eq!(sine_level(0.5), 231);
    let peaks = reduced(&sine(1000.0, 0.5, 8000));
    assert_eq!(peaks.buckets.len(), 100);
    for bucket in &peaks.buckets {
        assert_eq!(bucket[..3], [(-64i8) as u8, 64, 231], "{bucket:?}");
    }
    let peak = peaks.at(Time::from_seconds(0.5)).unwrap();
    assert_eq!((peak.min, peak.max, peak.rms), (-64, 64, 231));
    assert!((Peak::decibels(peak.rms) + 9.03).abs() < 0.2);
    assert_eq!((Peak::decibels(0), Peak::decibels(255)), (-96.0, 0.0));
    // Full scale stays inside the byte, and silence is nought.
    let loud = reduced(&sine(1000.0, 1.0, 800));
    assert_eq!(loud.buckets[5][..3], [(-128i8) as u8, 127, 247]);
    let silence = reduced(&vec![0; 1600]);
    assert_eq!(silence.buckets, vec![[0; 4]; 10]);
    assert_eq!(reduced(&[]).buckets.len(), 0);
}

#[test]
fn what_is_low_reads_high_in_the_low_level_and_what_is_high_reads_low() {
    let level = |bucket: &[u8; 4]| (i32::from(bucket[2]), i32::from(bucket[3]));
    // 60 Hz passes the low-pass almost whole. A bucket is 0.6 of its
    // period, so the level moves a little from one to the next.
    let low = reduced(&sine(60.0, 0.5, 8000));
    for bucket in &low.buckets[10..] {
        let (rms, under) = level(bucket);
        assert!((rms - 231).abs() <= 4, "{bucket:?}");
        assert!((0..=6).contains(&(rms - under)), "{bucket:?}");
    }
    // 2 kHz is 21.6 dB down behind it: 57 steps of the byte.
    let high = reduced(&sine(2000.0, 0.5, 8000));
    for bucket in &high.buckets[10..] {
        let (rms, under) = level(bucket);
        assert_eq!(rms, 231);
        assert!((55..=59).contains(&(rms - under)), "{bucket:?}");
    }
}

#[test]
fn sound_read_in_pieces_is_what_it_is_read_whole() {
    // Ten buckets and three samples of an eleventh.
    let bytes = sine(440.0, 0.3, 803);
    let whole = reduced(&bytes);
    assert_eq!(whole.buckets.len(), 11);
    // Pieces that end in the middle of a sample.
    for piece in [1, 3, 7, 161, 1000] {
        let mut reducer = Reducer::default();
        for part in bytes.chunks(piece) {
            reducer.push(part);
        }
        assert_eq!(reducer.len(), 10);
        assert_eq!(reducer.finish(), whole, "in pieces of {piece}");
    }
    // A byte left over is half a sample, and no sound.
    let mut odd = bytes.clone();
    odd.push(0x7f);
    assert_eq!(reduced(&odd), whole);
}

#[test]
fn a_bucket_is_found_by_its_source_time_and_a_stretch_by_what_it_touches() {
    let peaks = Peaks {
        buckets: (0..5i8)
            .map(|index| [(-index) as u8, (index * 10) as u8, index as u8, 0])
            .collect(),
    };
    assert_eq!(peaks.byte_len(), 20);
    let at = |micros: i64| peaks.at(Time(micros)).map(|peak| peak.rms);
    assert_eq!(at(0), Some(0));
    assert_eq!(at(9_999), Some(0));
    assert_eq!(at(10_000), Some(1));
    assert_eq!(at(49_999), Some(4));
    assert_eq!(at(50_000), None);
    assert_eq!(at(-1), None);

    let range = |from: i64, to: i64| peaks.range(Time(from), Time(to));
    // The end is not part of the stretch; either end may come first.
    assert_eq!(range(10_000, 30_000), Some((-2, 20)));
    assert_eq!(range(30_000, 10_000), Some((-2, 20)));
    assert_eq!(range(10_000, 30_001), Some((-3, 30)));
    // A stretch of no length is the bucket it is in.
    assert_eq!(range(25_000, 25_000), Some((-2, 20)));
    // What is outside the sound is left out, and all outside is nothing.
    assert_eq!(range(-50_000, 15_000), Some((-1, 10)));
    assert_eq!(range(35_000, 900_000), Some((-4, 40)));
    assert_eq!(range(-50_000, 0), None);
    assert_eq!(range(-50_000, -10_000), None);
    assert_eq!(range(50_000, 60_000), None);
    assert_eq!(Peaks::default().range(Time(0), Time(10_000)), None);
}

// ---- The cache -----------------------------------------------------------------

fn sound_at(path: &Path, bytes: usize) -> Asset {
    fs::write(path, vec![7u8; bytes]).unwrap();
    let mut asset = Asset::new(path.into(), MediaKind::Audio, Time::from_seconds(2.0));
    asset.file_size = bytes as u64;
    asset
}

fn some_peaks(buckets: usize) -> Peaks {
    Peaks {
        buckets: (0..buckets)
            .map(|index| [(-3i8) as u8, 5, (index % 251) as u8, 9])
            .collect(),
    }
}

#[test]
fn peaks_kept_are_read_back_until_their_media_changes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = PeakCache::at(dir.path().join("peaks"));
    let asset = sound_at(&dir.path().join("take.wav"), 100);
    // Nothing is kept, and asking makes no directory.
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    assert!(!cache.dir.exists());

    let peaks = some_peaks(200);
    cache.store(&asset, &Stamp::of(&asset), &peaks).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Held(Arc::new(peaks.clone())));
    assert_eq!(cache.held().1, 1);
    let name = cache.file(&asset);
    assert_eq!(name.extension().unwrap(), "peaks");
    assert_eq!(&fs::read(&name).unwrap()[..8], b"ASTRPEAK");
    // Another asset of the same file reads the same peaks: they are the
    // file's.
    let mut twin = asset.clone();
    twin.id = Uuid::new_v4();
    assert!(matches!(cache.load(&twin), KeptPeaks::Held(_)));

    // The project's own facts of the media are part of what it is.
    let mut longer = asset.clone();
    longer.duration = Time::from_seconds(3.0);
    assert_eq!(cache.load(&longer), KeptPeaks::Stale);
    let mut larger = asset.clone();
    larger.file_size += 1;
    assert_eq!(cache.load(&larger), KeptPeaks::Stale);
    // The file changed in place: another size, or only another time.
    fs::write(&asset.path, vec![7u8; 101]).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Stale);
    fs::write(&asset.path, vec![7u8; 100]).unwrap();
    let later = SystemTime::now() + Duration::from_secs(90);
    (fs::File::options().write(true).open(&asset.path).unwrap())
        .set_modified(later)
        .unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Stale);
    // And gone.
    fs::remove_file(&asset.path).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Stale);

    // Read again, they are the new file's.
    let asset = sound_at(&dir.path().join("take.wav"), 100);
    cache.store(&asset, &Stamp::of(&asset), &peaks).unwrap();
    assert!(matches!(cache.load(&asset), KeptPeaks::Held(_)));
    assert_eq!(cache.held().1, 1);
}

#[test]
fn a_damaged_file_or_one_of_other_peaks_is_no_peaks() {
    let dir = tempfile::tempdir().unwrap();
    let cache = PeakCache::at(dir.path().join("peaks"));
    let asset = sound_at(&dir.path().join("take.wav"), 100);
    let stamp = Stamp::of(&asset);
    cache.store(&asset, &stamp, &some_peaks(20)).unwrap();
    let file = cache.file(&asset);
    let whole = fs::read(&file).unwrap();
    // A file with its sum made right again: what is wrong is what it says.
    let summed = |mut bytes: Vec<u8>| {
        let body = bytes.len() - 8;
        let sum = fnv(&bytes[..body]);
        bytes[body..].copy_from_slice(&sum.to_le_bytes());
        bytes
    };

    // Cut short, one byte turned (the sum is wrong), grown, and empty.
    for damaged in [
        whole[..whole.len() - 5].to_vec(),
        {
            let mut bytes = whole.clone();
            bytes[whole.len() / 2] ^= 0x10;
            bytes
        },
        [whole.clone(), vec![0; 16]].concat(),
        Vec::new(),
    ] {
        fs::write(&file, damaged).unwrap();
        assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    }
    // Whole, but not a peak file.
    let mut other = whole.clone();
    other[..8].copy_from_slice(b"ASTRSTDY");
    assert_ne!(&other[..8], MAGIC);
    fs::write(&file, summed(other)).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    // Whole, but of another version of the reduction.
    let mut other = whole.clone();
    other[8..12].copy_from_slice(&(PEAK_ALGORITHM + 1).to_le_bytes());
    fs::write(&file, summed(other)).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    // Whole, but of the media at another path, which the file names.
    let elsewhere = sound_at(&dir.path().join("other.wav"), 100);
    let theirs = PeakCache::encode(&elsewhere, &Stamp::of(&elsewhere), &some_peaks(20));
    fs::write(&file, theirs).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    // Whole, with a bucket whose least sample is over its greatest.
    let mut wrong = some_peaks(20);
    wrong.buckets[4] = [9, 2, 0, 0];
    fs::write(&file, PeakCache::encode(&asset, &stamp, &wrong)).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    // Whole, with no bucket at all.
    fs::write(&file, PeakCache::encode(&asset, &stamp, &Peaks::default())).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    // Larger than any peaks may be: not read at all.
    fs::write(&file, vec![0u8; 4097 + MAX_PEAK_BUCKETS * 4]).unwrap();
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    fs::write(&file, whole).unwrap();
    assert!(matches!(cache.load(&asset), KeptPeaks::Held(_)));
}

#[test]
fn the_cache_is_held_to_its_bounds_and_gives_up_what_was_read_longest_ago() {
    assert_eq!(
        (PEAK_CACHE_BYTES, PEAK_CACHE_ENTRIES, MAX_PEAK_BUCKETS),
        (64 * 1024 * 1024, 512, 1 << 21)
    );
    let dir = tempfile::tempdir().unwrap();
    let assets: Vec<Asset> = (0..6)
        .map(|index| sound_at(&dir.path().join(format!("take{index}.wav")), 50 + index))
        .collect();
    let peaks = some_peaks(100);
    let one = PeakCache::encode(&assets[0], &Stamp::of(&assets[0]), &peaks).len() as u64;
    // Room for three files by their count.
    let cache = PeakCache {
        dir: dir.path().join("peaks"),
        bytes: u64::MAX,
        entries: 3,
    };
    let aged = |file: &Path, seconds: u64| {
        let at = SystemTime::now() - Duration::from_secs(1000 - seconds);
        (fs::File::options().write(true).open(file).unwrap())
            .set_modified(at)
            .unwrap();
    };
    for (index, asset) in assets[..3].iter().enumerate() {
        cache.store(asset, &Stamp::of(asset), &peaks).unwrap();
        aged(&cache.file(asset), index as u64);
    }
    assert_eq!(cache.held(), (3 * one, 3));
    // The oldest is read, and so is the newest of them now.
    assert!(matches!(cache.load(&assets[0]), KeptPeaks::Held(_)));
    let read = fs::metadata(cache.file(&assets[0]))
        .unwrap()
        .modified()
        .unwrap();
    assert!(read.elapsed().unwrap() < Duration::from_secs(100));
    aged(&cache.file(&assets[0]), 500);
    cache
        .store(&assets[3], &Stamp::of(&assets[3]), &peaks)
        .unwrap();
    aged(&cache.file(&assets[3]), 600);
    assert_eq!(cache.held().1, 3);
    assert!(cache.file(&assets[0]).exists());
    assert!(!cache.file(&assets[1]).exists(), "the one read longest ago");
    assert!(cache.file(&assets[2]).exists());
    assert!(cache.file(&assets[3]).exists());

    // Room for two files by their bytes: the two read longest ago go.
    let cache = PeakCache {
        bytes: 2 * one + one / 2,
        entries: 100,
        ..cache
    };
    cache
        .store(&assets[4], &Stamp::of(&assets[4]), &peaks)
        .unwrap();
    let (bytes, files) = cache.held();
    assert!(
        bytes <= cache.bytes && files == 2,
        "{bytes} bytes in {files}"
    );
    assert!(
        cache.file(&assets[4]).exists(),
        "what was just written stays"
    );
    assert!(cache.file(&assets[3]).exists());
    assert!(!cache.file(&assets[2]).exists());
    // A file larger than all the room is still kept: it is all there is.
    let cache = PeakCache { bytes: 10, ..cache };
    cache
        .store(&assets[5], &Stamp::of(&assets[5]), &peaks)
        .unwrap();
    assert_eq!(cache.held(), (one, 1));
    assert!(matches!(cache.load(&assets[5]), KeptPeaks::Held(_)));
    // Nothing but the cache's own files is touched: not a stranger's, and
    // not a write still under way beside its place. One that never ended
    // is swept an hour on.
    fs::write(cache.dir.join("notes.txt"), b"mine").unwrap();
    let (fresh, left) = (
        cache.dir.join("0123.peaks.aaaa"),
        cache.dir.join("0123.peaks.bbbb"),
    );
    fs::write(&fresh, b"half").unwrap();
    fs::write(&left, b"half").unwrap();
    (fs::File::options().write(true).open(&left).unwrap())
        .set_modified(SystemTime::now() - Duration::from_secs(2 * 3600))
        .unwrap();
    cache
        .store(&assets[0], &Stamp::of(&assets[0]), &peaks)
        .unwrap();
    assert!(cache.dir.join("notes.txt").exists());
    assert!(fresh.exists());
    assert!(!left.exists());
    assert_eq!(cache.held().1, 1);
}

// ---- The lane ----------------------------------------------------------------

/// A lane whose reading waits for the gate, or for being given up, and
/// whose beats, found in what the cache keeps, wait for a gate of their own
/// that is open until a test shuts it.
struct Gated {
    service: SoundService,
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Arc<AtomicUsize>,
    beat_gate: Arc<(Mutex<bool>, Condvar)>,
    /// Beat findings begun, and ended.
    finding: Arc<AtomicUsize>,
    found: Arc<AtomicUsize>,
    /// What a written file waits for once it has claimed its last step,
    /// open until a test shuts it, and how many have claimed it.
    place_gate: Arc<(Mutex<bool>, Condvar)>,
    claimed: Arc<AtomicUsize>,
    wakes: Arc<AtomicUsize>,
    dir: tempfile::TempDir,
    asset: Asset,
    cache: PeakCache,
}

impl Gated {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let asset = sound_at(&dir.path().join("take.wav"), 64);
        let cache = PeakCache::at(dir.path().join("peaks"));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let started = Arc::new(AtomicUsize::new(0));
        let wakes = Arc::new(AtomicUsize::new(0));
        // Both kinds of job say how far they are and wait at the same gate.
        let waits = {
            let (held, begun) = (Arc::clone(&gate), Arc::clone(&started));
            move |control: &Control| {
                begun.fetch_add(1, Ordering::AcqRel);
                control.progress(0.25);
                control.progress(0.5);
                let mut open = held.0.lock().unwrap();
                while !*open {
                    if control.cancelled() {
                        return Err(MediaError::Cancelled);
                    }
                    open = (held.1)
                        .wait_timeout(open, Duration::from_millis(2))
                        .unwrap()
                        .0;
                }
                Ok(())
            }
        };
        let (measures, saves) = (waits.clone(), waits.clone());
        let place_gate = Arc::new((Mutex::new(true), Condvar::new()));
        let claimed = Arc::new(AtomicUsize::new(0));
        let writes = {
            let (held, placing) = (Arc::clone(&place_gate), Arc::clone(&claimed));
            move |job: &ExtractJob, control: &Control| {
                saves(control)?;
                if !control.claim() {
                    return Err(MediaError::Cancelled);
                }
                placing.fetch_add(1, Ordering::AcqRel);
                // Claimed: nothing gives it up now, so it waits for no one.
                let mut open = held.0.lock().unwrap();
                while !*open {
                    open = held.1.wait(open).unwrap();
                }
                // One named so fails at its last step, as a rename may.
                if job.to.ends_with("fails.wav") {
                    return Err(MediaError::Invalid("Cannot write fails.wav".into()));
                }
                // A file reads as long as its span, to tell answers apart.
                Ok(job.span)
            }
        };
        let beat_gate = Arc::new((Mutex::new(true), Condvar::new()));
        let (finding, found) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let finds = {
            let (held, begun, ended) = (
                Arc::clone(&beat_gate),
                Arc::clone(&finding),
                Arc::clone(&found),
            );
            move |job: &PeakJob| {
                begun.fetch_add(1, Ordering::AcqRel);
                let mut open = held.0.lock().unwrap();
                while !*open {
                    open = held.1.wait(open).unwrap();
                }
                drop(open);
                let result = find_beats(job);
                ended.fetch_add(1, Ordering::AcqRel);
                result
            }
        };
        let counted = Arc::clone(&wakes);
        let service = SoundService::with_runners(
            Arc::new(move || {
                counted.fetch_add(1, Ordering::AcqRel);
            }),
            Processes::default(),
            Runners {
                peaks: Arc::new(move |job: &PeakJob, control: &Control| {
                    waits(control)?;
                    let peaks = Arc::new(some_peaks(job.asset.file_size as usize));
                    let unkept = (job.cache)
                        .store(&job.asset, &Stamp::of(&job.asset), &peaks)
                        .err();
                    Ok((peaks, unkept))
                }),
                // A span reads as loud as it is long, to tell answers apart.
                loudness: Arc::new(move |job: &LoudnessJob, control: &Control| {
                    measures(control)?;
                    Ok(Loudness::Measured {
                        integrated: -(job.span.as_seconds() as f32),
                        true_peak: -(job.from.as_seconds() as f32),
                    })
                }),
                extract: Arc::new(writes),
                separate: Arc::new(|_: &SeparateJob, _: &Control| {
                    Err(MediaError::Invalid("No separator in this bench".into()))
                }),
                beats: Arc::new(finds),
            },
        )
        .unwrap();
        Self {
            service,
            gate,
            started,
            beat_gate,
            finding,
            found,
            place_gate,
            claimed,
            wakes,
            dir,
            asset,
            cache,
        }
    }

    fn job(&self, project: u128) -> PeakJob {
        PeakJob {
            project: Uuid::from_u128(project),
            asset: self.asset.clone(),
            cache: self.cache.clone(),
        }
    }

    /// A job for another media file, `bytes` long.
    fn another(&self, bytes: usize) -> PeakJob {
        PeakJob {
            asset: sound_at(&self.dir.path().join(format!("other{bytes}.wav")), bytes),
            ..self.job(1)
        }
    }

    /// A span of the gated media to measure, with no effects.
    fn span(&self, project: u128, from: f64, span: f64) -> LoudnessJob {
        LoudnessJob {
            project: Uuid::from_u128(project),
            asset: self.asset.clone(),
            from: Time::from_seconds(from),
            span: Time::from_seconds(span),
            effects: Vec::new(),
        }
    }

    /// A span of the gated media to write, to a file named for its start.
    fn save(&self, project: u128, from: f64, span: f64) -> ExtractJob {
        ExtractJob {
            project: Uuid::from_u128(project),
            asset: self.asset.clone(),
            from: Time::from_seconds(from),
            span: Time::from_seconds(span),
            to: self.dir.path().join(format!("take{from}.wav")),
            keep: Vec::new(),
        }
    }

    fn open(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }

    fn hold_placing(&self, held: bool) {
        *self.place_gate.0.lock().unwrap() = !held;
        self.place_gate.1.notify_all();
    }

    fn hold_beats(&self, held: bool) {
        *self.beat_gate.0.lock().unwrap() = !held;
        self.beat_gate.1.notify_all();
    }

    fn wait(&self, what: &str, mut until: impl FnMut() -> bool) {
        let limit = Instant::now() + Duration::from_secs(10);
        while !until() {
            assert!(Instant::now() < limit, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn started(&self, count: usize) {
        self.wait("the job to start", || {
            self.started.load(Ordering::Acquire) == count
        });
    }

    fn events(&self) -> Vec<SoundEvent> {
        self.wait("the lane to be idle", || !self.service.busy());
        std::iter::from_fn(|| self.service.poll()).collect()
    }

    /// The requests that answered with peaks, a measurement or a written
    /// file, in order.
    fn answered(&self) -> Vec<u64> {
        (self.events().into_iter())
            .filter_map(|event| match event {
                SoundEvent::Peaked { request, .. }
                | SoundEvent::Measured { request, .. }
                | SoundEvent::Extracted { request, .. } => Some(request),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_reading_says_how_far_it_is_and_answers_with_what_it_read() {
    let lane = Gated::new();
    // Nobody is woken by a lane nothing was asked of.
    thread::sleep(Duration::from_millis(30));
    assert_eq!(lane.wakes.load(Ordering::Acquire), 0);

    let request = lane.service.peaks(lane.job(7)).unwrap();
    lane.started(1);
    lane.wait("its progress", || lane.wakes.load(Ordering::Acquire) >= 2);
    // One figure waits, the newest.
    match lane.service.poll() {
        Some(SoundEvent::Progress { request: of, done }) => {
            assert_eq!((of, done), (request, 0.5));
        }
        other => panic!("{other:?}"),
    }
    assert!(lane.service.poll().is_none());
    lane.open();
    let events = lane.events();
    let [
        SoundEvent::Peaked {
            request: of,
            project,
            asset_id,
            source,
            result: Ok(peaks),
            unkept: None,
        },
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert_eq!((*of, *project), (request, Uuid::from_u128(7)));
    assert_eq!(*asset_id, lane.asset.id);
    assert_eq!(*source, SourceIdentity::of(&lane.asset));
    assert_eq!(peaks.buckets.len(), 64);
    // And they were kept.
    assert_eq!(
        lane.cache.load(&lane.asset),
        KeptPeaks::Held(Arc::clone(peaks))
    );
    // Two figures and the answer woke the window, and an idle lane wakes
    // nobody after.
    assert_eq!(lane.wakes.load(Ordering::Acquire), 3);
    thread::sleep(Duration::from_millis(30));
    assert_eq!(lane.wakes.load(Ordering::Acquire), 3);
}

#[test]
fn a_reading_given_up_or_replaced_answers_nothing() {
    let lane = Gated::new();
    let first = lane.service.peaks(lane.job(1)).unwrap();
    lane.started(1);
    lane.service.cancel(first);
    assert!(lane.events().is_empty());
    assert_eq!(lane.cache.load(&lane.asset), KeptPeaks::Missing);

    // A second for the same media takes the first's place under way, and
    // what the first had said by then and nobody read goes with it.
    let before = lane.wakes.load(Ordering::Acquire);
    let first = lane.service.peaks(lane.job(1)).unwrap();
    lane.started(2);
    lane.wait("its progress", || {
        lane.wakes.load(Ordering::Acquire) >= before + 2
    });
    let second = lane.service.peaks(lane.job(2)).unwrap();
    assert_ne!(first, second);
    // And a third takes the second's while it waits. Other media waits its
    // turn through all of it.
    let other = lane.service.peaks(lane.another(8)).unwrap();
    let third = lane.service.peaks(lane.job(3)).unwrap();
    lane.open();
    // Nothing of any kind is left of the two that were replaced, nor a
    // figure of a job that has answered.
    let done: Vec<(u64, Uuid)> = (lane.events().into_iter())
        .map(|event| match event {
            SoundEvent::Peaked {
                request, project, ..
            } => (request, project),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        done,
        vec![(other, Uuid::from_u128(1)), (third, Uuid::from_u128(3))]
    );
    // The first was stopped. The second began only if the worker came to
    // it before the third took its place, and then it was stopped as well.
    let begun = lane.started.load(Ordering::Acquire);
    assert!((4..=5).contains(&begun), "{begun} began");

    // One that waits is given up by its name, and is never begun.
    *lane.gate.0.lock().unwrap() = false;
    let under_way = lane.service.peaks(lane.job(4)).unwrap();
    lane.started(begun + 1);
    let waiting = lane.service.peaks(lane.another(9)).unwrap();
    lane.service.cancel(waiting);
    lane.open();
    assert_eq!(lane.answered(), vec![under_way]);
    assert_eq!(lane.started.load(Ordering::Acquire), begun + 1);

    // Another project: everything is given up at once.
    *lane.gate.0.lock().unwrap() = false;
    lane.service.peaks(lane.job(5)).unwrap();
    lane.service.peaks(lane.another(10)).unwrap();
    lane.service.load_peaks(lane.job(5)).unwrap();
    lane.service.cancel_all();
    assert!(lane.events().is_empty());
}

#[test]
fn jobs_of_different_targets_leave_each_other_alone() {
    let lane = Gated::new();
    let peaks = lane.service.peaks(lane.job(1)).unwrap();
    lane.started(1);
    // A measurement of the same media, while its peaks are read: it waits,
    // and stops nothing.
    let measured = lane.service.loudness(lane.span(1, 0.0, 1.0)).unwrap();
    // A newer of the same span takes that one's place, and still not the
    // peaks'.
    let newer = lane.service.loudness(lane.span(2, 0.0, 1.0)).unwrap();
    assert_ne!(measured, newer);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(lane.started.load(Ordering::Acquire), 1);
    lane.open();
    assert_eq!(lane.answered(), vec![peaks, newer]);
    assert_eq!(lane.started.load(Ordering::Acquire), 2);

    // The other way round: peaks asked for while a measurement is under
    // way leave it answering.
    *lane.gate.0.lock().unwrap() = false;
    let measured = lane.service.loudness(lane.span(1, 0.0, 1.0)).unwrap();
    lane.started(3);
    let peaks = lane.service.peaks(lane.job(1)).unwrap();
    lane.open();
    assert_eq!(lane.answered(), vec![measured, peaks]);
}

#[test]
fn two_spans_of_one_media_both_answer_and_the_same_span_again_replaces_it() {
    let lane = Gated::new();
    let first = lane.service.loudness(lane.span(1, 0.0, 1.0)).unwrap();
    lane.started(1);
    // Another span, under way or waiting, is another target: one that
    // starts elsewhere and one that starts there and is longer.
    let later = lane.service.loudness(lane.span(1, 1.0, 1.0)).unwrap();
    let longer = lane.service.loudness(lane.span(1, 0.0, 2.0)).unwrap();
    // The same span again takes the place of the one that waits, and only
    // that one, with what it had said.
    let again = lane.service.loudness(lane.span(2, 1.0, 1.0)).unwrap();
    lane.open();
    let events = lane.events();
    let answers: Vec<(u64, Uuid, AssetId, Result<Loudness, String>)> = (events.into_iter())
        .map(|event| match event {
            SoundEvent::Measured {
                request,
                project,
                asset_id,
                source,
                result,
            } => {
                assert_eq!(source, SourceIdentity::of(&lane.asset));
                (request, project, asset_id, result)
            }
            other => panic!("{other:?}"),
        })
        .collect();
    let measured = |integrated: f32, true_peak: f32| {
        Ok(Loudness::Measured {
            integrated,
            true_peak,
        })
    };
    let id = lane.asset.id;
    assert_eq!(
        answers,
        vec![
            (first, Uuid::from_u128(1), id, measured(-1.0, -0.0)),
            (longer, Uuid::from_u128(1), id, measured(-2.0, -0.0)),
            (again, Uuid::from_u128(2), id, measured(-1.0, -1.0)),
        ]
    );
    assert!(!answers.iter().any(|answer| answer.0 == later));
    assert_eq!(lane.started.load(Ordering::Acquire), 3);

    // And the one under way: the same span again stops it, and it says
    // nothing.
    *lane.gate.0.lock().unwrap() = false;
    let stopped = lane.service.loudness(lane.span(3, 0.5, 0.5)).unwrap();
    lane.started(4);
    let replacing = lane.service.loudness(lane.span(4, 0.5, 0.5)).unwrap();
    lane.started(5);
    lane.open();
    assert_eq!(lane.answered(), vec![replacing]);
    assert_ne!(stopped, replacing);
}

#[test]
fn the_seventeenth_waiting_request_is_busy() {
    assert_eq!((SOUND_QUEUE, PEAK_LOADS), (16, 16));
    let lane = Gated::new();
    let under_way = lane.service.peaks(lane.job(1)).unwrap();
    lane.started(1);
    let waiting: Vec<(u64, PeakJob)> = (0..SOUND_QUEUE)
        .map(|index| {
            let job = lane.another(index + 1);
            (lane.service.peaks(job.clone()).unwrap(), job)
        })
        .collect();
    assert!(matches!(
        lane.service.peaks(lane.another(100)),
        Err(MediaError::Busy)
    ));
    // The one under way is not among those that wait: a newer for its
    // media has no room either, and does not stop it for nothing.
    assert!(matches!(
        lane.service.peaks(lane.job(2)),
        Err(MediaError::Busy)
    ));
    // A newer for media that waits takes its place, and needs no room.
    let (older, again) = waiting[3].clone();
    let newer = lane.service.peaks(again).unwrap();
    assert_ne!(older, newer);
    // Kept peaks are asked for in a queue of their own.
    let asked = lane.service.load_peaks(lane.another(200)).unwrap();
    lane.open();
    let events = lane.events();
    let mut read = Vec::new();
    let mut loaded = Vec::new();
    for event in events {
        match event {
            SoundEvent::Peaked {
                request,
                result: Ok(_),
                ..
            } => read.push(request),
            SoundEvent::Loaded { request, kept, .. } => loaded.push((request, kept)),
            other => panic!("{other:?}"),
        }
    }
    let mut expected: Vec<u64> = std::iter::once(under_way)
        .chain(waiting.iter().map(|(request, _)| *request))
        .filter(|request| *request != older)
        .collect();
    expected.push(newer);
    assert_eq!(read, expected);
    assert_eq!(loaded, vec![(asked, KeptPeaks::Missing)]);
    assert_eq!(lane.started.load(Ordering::Acquire), 1 + SOUND_QUEUE);
}

#[test]
fn kept_peaks_are_read_without_waiting_for_sound_being_read() {
    let lane = Gated::new();
    let other = lane.another(32).asset;
    let kept = some_peaks(12);
    (lane.cache)
        .store(&other, &Stamp::of(&other), &kept)
        .unwrap();

    let reading_sound = lane.service.peaks(lane.job(5)).unwrap();
    lane.started(1);
    let reading = (lane.service)
        .load_peaks(PeakJob {
            asset: other.clone(),
            ..lane.job(5)
        })
        .unwrap();
    let missing = lane.service.load_peaks(lane.job(5)).unwrap();
    // Both are answered while the sound still waits at its gate.
    let mut answers = Vec::new();
    lane.wait("the readings", || {
        while let Some(event) = lane.service.poll() {
            if let SoundEvent::Loaded {
                request,
                project,
                asset_id,
                source,
                kept,
            } = event
            {
                assert_eq!(project, Uuid::from_u128(5));
                answers.push((request, asset_id, kept, source));
            }
        }
        answers.len() == 2
    });
    assert_eq!(lane.started.load(Ordering::Acquire), 1);
    assert_eq!(
        answers,
        vec![
            (
                reading,
                other.id,
                KeptPeaks::Held(Arc::new(kept)),
                SourceIdentity::of(&other)
            ),
            (
                missing,
                lane.asset.id,
                KeptPeaks::Missing,
                SourceIdentity::of(&lane.asset)
            ),
        ]
    );
    // The file changed since: what is kept of it is said to be stale.
    fs::write(&other.path, vec![7u8; 33]).unwrap();
    let stale = (lane.service)
        .load_peaks(PeakJob {
            asset: other.clone(),
            ..lane.job(5)
        })
        .unwrap();
    let mut said = None;
    lane.wait("the stale reading", || {
        while let Some(event) = lane.service.poll() {
            if let SoundEvent::Loaded { request, kept, .. } = event {
                said = Some((request, kept));
            }
        }
        said.is_some()
    });
    assert_eq!(said, Some((stale, KeptPeaks::Stale)));
    lane.service.cancel(reading_sound);
    assert!(lane.events().is_empty());
}

#[test]
fn media_without_sound_or_too_long_is_refused_before_any_process() {
    let _heavy = crate::heavy_test();
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    let job = |asset: Asset| PeakJob {
        project: Uuid::from_u128(1),
        asset,
        cache: PeakCache::at(dir.path().join("peaks")),
    };
    let refused = |asset: Asset, why: &str| {
        for asked in [
            service.peaks(job(asset.clone())),
            service.load_peaks(job(asset.clone())),
        ] {
            match asked {
                Err(MediaError::Invalid(said)) => assert!(said.contains(why), "{said}"),
                other => panic!("{other:?}"),
            }
        }
    };
    // A video with no sound stream, and a still.
    let mut silent = Asset::new(
        "silent.mp4".into(),
        MediaKind::Video,
        Time::from_seconds(4.0),
    );
    (silent.width, silent.height) = (320, 240);
    refused(silent.clone(), "no sound");
    refused(
        Asset::new(
            "still.png".into(),
            MediaKind::Image,
            Time::from_seconds(5.0),
        ),
        "no sound",
    );
    // More buckets than a file of peaks may hold: one over the bound.
    let longest = Time(MAX_PEAK_BUCKETS as i64 * 10_000);
    let over = Asset::new("day.wav".into(), MediaKind::Audio, Time(longest.0 + 1));
    refused(over, "too long");
    refused(
        Asset::new("none.wav".into(), MediaKind::Audio, Time::ZERO),
        "no length",
    );
    assert_eq!(service.processes.started_tools(), 0);
    assert!(!service.busy());
    assert!(service.poll().is_none());

    // The bound itself is asked: the job starts, and says its file is not
    // there. So is a video with sound.
    let gone = Asset::new(dir.path().join("gone.wav"), MediaKind::Audio, longest);
    let mut asked = vec![service.peaks(job(gone)).unwrap()];
    silent.has_audio = true;
    silent.path = dir.path().join("gone.mp4");
    asked.push(service.peaks(job(silent)).unwrap());
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; a missing sound file was not asked of it");
        return;
    }
    for request in asked {
        let (result, unkept) = await_peaked(&service, request);
        assert!(result.is_err() && unkept.is_none(), "{result:?}");
    }
    assert_eq!(service.processes.started_tools(), 2);
}

// ---- FFmpeg ----------------------------------------------------------------------

fn tools_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        tool_command(tool)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Runs FFmpeg to make a fixture at `path` from the arguments before it.
fn made(path: &Path, arguments: &[&str]) {
    let status = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(arguments)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "could not make {}", path.display());
}

/// The asset an import would make of a file: its length is the container's.
fn probed(path: &Path) -> Asset {
    let mut command = tool_command("ffprobe");
    command
        .args(["-v", "error", "-show_entries"])
        .arg("format=duration:stream=codec_type,width,height,duration,avg_frame_rate")
        .args(["-of", "json"])
        .arg(path);
    let bytes = capture("ffprobe", &mut command, &Processes::default(), 1 << 20).unwrap();
    let mut asset = asset_from_probe(path, &bytes).unwrap();
    asset.file_size = fs::metadata(path).unwrap().len();
    asset
}

/// A sine at half of full scale, one channel of 16-bit samples at 48 kHz.
fn tone(dir: &Path, hertz: u32, seconds: u32) -> Asset {
    let path = dir.join(format!("tone{hertz}.wav"));
    let source = format!("aevalsrc=0.5*sin(2*PI*{hertz}*t):s=48000:d={seconds}");
    made(&path, &["-f", "lavfi", "-i", &source, "-c:a", "pcm_s16le"]);
    probed(&path)
}

fn await_peaked(
    service: &SoundService,
    request: u64,
) -> (Result<Arc<Peaks>, String>, Option<String>) {
    let limit = Instant::now() + Duration::from_secs(120);
    let mut said = 0.0;
    loop {
        match service.poll() {
            Some(SoundEvent::Progress { request: of, done }) if of == request => {
                assert!(done >= said, "progress went back from {said} to {done}");
                assert!(done <= 1.0, "progress went past the end: {done}");
                said = done;
            }
            Some(SoundEvent::Peaked {
                request: of,
                result,
                unkept,
                ..
            }) if of == request => return (result, unkept),
            Some(_) => {}
            None => {
                assert!(Instant::now() < limit, "timed out waiting for the peaks");
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// The peaks of a file, read by a lane of its own and kept in `cache`.
fn read(asset: &Asset, cache: &PeakCache) -> Arc<Peaks> {
    let service = SoundService::new(|| {}).unwrap();
    let request = service
        .peaks(PeakJob {
            project: Uuid::from_u128(1),
            asset: asset.clone(),
            cache: cache.clone(),
        })
        .unwrap();
    let (result, unkept) = await_peaked(&service, request);
    assert_eq!(unkept, None);
    assert_eq!(service.processes.started_tools(), 1);
    result.unwrap()
}

/// Whether a file's bucket count is its container's length, within one.
fn counted(peaks: &Peaks, asset: &Asset) {
    let expected = (asset.duration.as_seconds() * f64::from(PEAKS_PER_SECOND)).round() as i64;
    let count = peaks.buckets.len() as i64;
    assert!(
        (count - expected).abs() <= 1,
        "{count} buckets of {} s",
        asset.duration.as_seconds()
    );
}

#[test]
fn tones_and_silence_in_files_are_read_at_their_level() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the tone and silence fixtures were not read");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cache = PeakCache::at(dir.path().join("peaks"));
    // Away from the file's two ends, where the resampler's filter is not
    // yet full, and from the last bucket, which may be part of one.
    let middle = |peaks: &Peaks| peaks.buckets[10..peaks.buckets.len() - 10].to_vec();
    let level = |bucket: &[u8; 4]| (i32::from(bucket[2]), i32::from(bucket[3]));

    // 1 kHz at half of full scale: -9.03 dBFS is byte 231, and the peaks
    // are 64 128ths either way.
    let asset = tone(dir.path(), 1000, 3);
    let peaks = read(&asset, &cache);
    counted(&peaks, &asset);
    for bucket in middle(&peaks) {
        let (min, max) = (i32::from(bucket[0] as i8), i32::from(bucket[1] as i8));
        assert!((level(&bucket).0 - 231).abs() <= 1, "{bucket:?}");
        assert!((min + 64).abs() <= 1 && (max - 64).abs() <= 1, "{bucket:?}");
    }
    // What was read is what is kept, for this asset and no changed one.
    assert_eq!(cache.load(&asset), KeptPeaks::Held(Arc::clone(&peaks)));
    assert_eq!(cache.held().1, 1);

    // 60 Hz is under the low-pass, 2 kHz far over it.
    let asset = tone(dir.path(), 60, 2);
    let peaks = read(&asset, &cache);
    counted(&peaks, &asset);
    for bucket in middle(&peaks) {
        let (rms, low) = level(&bucket);
        assert!((rms - 231).abs() <= 5, "{bucket:?}");
        assert!((0..=6).contains(&(rms - low)), "{bucket:?}");
    }
    let asset = tone(dir.path(), 2000, 2);
    let peaks = read(&asset, &cache);
    counted(&peaks, &asset);
    for bucket in middle(&peaks) {
        let (rms, low) = level(&bucket);
        assert!((rms - 231).abs() <= 1, "{bucket:?}");
        assert!((54..=60).contains(&(rms - low)), "{bucket:?}");
    }

    // Silence is nought throughout, in two channels as in one.
    let path = dir.path().join("silence.wav");
    made(
        &path,
        &["-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo", "-t", "2"],
    );
    let asset = probed(&path);
    let peaks = read(&asset, &cache);
    counted(&peaks, &asset);
    assert!(peaks.buckets.iter().all(|bucket| *bucket == [0; 4]));
    assert_eq!(cache.held().1, 4);
}

#[test]
fn sound_that_starts_late_in_its_file_keeps_its_place_in_container_time() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the late-starting fixture was not read");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Three seconds of picture, and two of a tone that begin a second in.
    let path = dir.path().join("late.mp4");
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x48:r=30:d=3",
            "-itsoffset",
            "1",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.5*sin(2*PI*1000*t):s=48000:d=2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ],
    );
    let asset = probed(&path);
    assert!(asset.kind == MediaKind::Video && asset.has_audio);
    let peaks = read(&asset, &PeakCache::at(dir.path().join("peaks")));
    // The count is the container's, not the two seconds of the stream.
    counted(&peaks, &asset);
    assert!(peaks.buckets.len() >= 299, "{}", peaks.buckets.len());
    // Loud is within 12 dB of the tone's own level.
    let first = (peaks.buckets.iter())
        .position(|bucket| bucket[2] >= 231 - 32)
        .expect("the tone is in the file");
    eprintln!(
        "MEASURE: late fixture: {} buckets, first loud bucket {first}",
        peaks.buckets.len()
    );
    assert!((97..=103).contains(&first), "first loud bucket {first}");
    // Before it there is nothing, and in it the tone at its level.
    assert!(peaks.buckets[..90].iter().all(|bucket| *bucket == [0; 4]));
    let peak = peaks.at(Time::from_seconds(2.0)).unwrap();
    assert!((i32::from(peak.rms) - 231).abs() <= 2, "{peak:?}");
    assert_eq!(peaks.at(Time::from_seconds(0.5)).unwrap().rms, 0);
}

/// Two hours of silence, which FFmpeg makes in under a second and the lane
/// reads in about one: long enough to be given up while it is read.
fn long(dir: &Path) -> Asset {
    let path = dir.join("long.flac");
    made(
        &path,
        &["-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-t", "7200"],
    );
    probed(&path)
}

/// Waits for a job's first word, a hundredth of the way in.
fn await_progress(service: &SoundService, request: u64) {
    let limit = Instant::now() + Duration::from_secs(60);
    loop {
        match service.poll() {
            Some(SoundEvent::Progress { request: of, done }) if of == request => {
                assert!(done < 0.5, "the job was to be caught early, not at {done}");
                return;
            }
            Some(other) => panic!("{other:?}"),
            None => {
                assert!(Instant::now() < limit, "the job said nothing");
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

/// Waits until the lane is idle and every process it started is reaped:
/// a process is waited for when it is dropped, and its watchdog ends then.
fn await_reaped(service: &SoundService) {
    let limit = Instant::now() + Duration::from_secs(20);
    while service.busy() || service.processes.retained_monitors() != 0 {
        assert!(Instant::now() < limit, "the job did not stop");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_reading_of_a_file_stops_when_it_is_given_up_or_replaced() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; cancelling a sound process was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let asset = long(dir.path());
    let cache = PeakCache::at(dir.path().join("peaks"));
    let wakes = Arc::new(AtomicUsize::new(0));
    let woke = Arc::clone(&wakes);
    let service = SoundService::new(move || {
        woke.fetch_add(1, Ordering::AcqRel);
    })
    .unwrap();
    let job = |project: u128| PeakJob {
        project: Uuid::from_u128(project),
        asset: asset.clone(),
        cache: cache.clone(),
    };

    // Given up at its first word.
    let asked = service.peaks(job(1)).unwrap();
    await_progress(&service, asked);
    service.cancel(asked);
    await_reaped(&service);
    assert!(service.poll().is_none());
    assert_eq!(cache.held().1, 0);
    assert_eq!(service.processes.started_tools(), 1);

    // Replaced at its first word: the first is stopped and says nothing,
    // the second reads the whole file.
    let first = service.peaks(job(2)).unwrap();
    await_progress(&service, first);
    let before = wakes.load(Ordering::Acquire);
    let began = Instant::now();
    let second = service.peaks(job(3)).unwrap();
    let limit = Instant::now() + Duration::from_secs(120);
    let peaks = loop {
        match service.poll() {
            Some(SoundEvent::Progress { request, .. }) => assert_eq!(request, second),
            Some(SoundEvent::Peaked {
                request,
                project,
                result,
                unkept,
                ..
            }) => {
                assert_eq!((request, project), (second, Uuid::from_u128(3)));
                assert_eq!(unkept, None);
                break result.unwrap();
            }
            Some(other) => panic!("{other:?}"),
            None => {
                assert!(Instant::now() < limit, "timed out waiting for the peaks");
                thread::sleep(Duration::from_millis(1));
            }
        }
    };
    eprintln!(
        "MEASURE: {} buckets of {} s of silent FLAC read in {:.2} s",
        peaks.buckets.len(),
        asset.duration.as_seconds(),
        began.elapsed().as_secs_f64()
    );
    await_reaped(&service);
    assert!(service.poll().is_none());
    assert_eq!(service.processes.started_tools(), 3);
    counted(&peaks, &asset);
    assert!(matches!(cache.load(&asset), KeptPeaks::Held(_)));
    // A wake for each whole percent and one for the answer, at most: the
    // first job may have said one more word before it was stopped.
    let woken = wakes.load(Ordering::Acquire) - before;
    assert!((2..=102).contains(&woken), "{woken} wakes");
    // And none from a lane with nothing to do.
    let idle = wakes.load(Ordering::Acquire);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(wakes.load(Ordering::Acquire), idle);

    // Every request given up at once, as when the project is replaced.
    let asked = service.peaks(job(4)).unwrap();
    await_progress(&service, asked);
    service.load_peaks(job(4)).unwrap();
    service.cancel_all();
    await_reaped(&service);
    assert!(service.poll().is_none());
    assert_eq!(service.processes.started_tools(), 4);
}

#[test]
fn peaks_that_cannot_be_kept_are_still_the_answer() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; a failed peak store was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let asset = tone(dir.path(), 1000, 1);
    // The cache's directory cannot be made: a file is in its way.
    let blocked = dir.path().join("in-the-way");
    fs::write(&blocked, b"a file").unwrap();
    let cache = PeakCache::at(blocked.join("peaks"));
    let service = SoundService::new(|| {}).unwrap();
    let job = PeakJob {
        project: Uuid::from_u128(1),
        asset: asset.clone(),
        cache: cache.clone(),
    };
    let request = service.peaks(job.clone()).unwrap();
    let (result, unkept) = await_peaked(&service, request);
    counted(&result.unwrap(), &asset);
    let unkept = unkept.expect("the store failed");
    assert!(unkept.contains("in-the-way"), "{unkept}");
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    assert_eq!(fs::read(&blocked).unwrap(), b"a file");

    // A file that is no sound is an answer too, in FFmpeg's words, and
    // nothing of it is kept.
    let cache = PeakCache::at(dir.path().join("peaks"));
    let noise = sound_at(&dir.path().join("noise.wav"), 4096);
    let request = service
        .peaks(PeakJob {
            asset: noise.clone(),
            cache: cache.clone(),
            ..job
        })
        .unwrap();
    let (result, unkept) = await_peaked(&service, request);
    let said = result.unwrap_err();
    assert!(said.starts_with("ffmpeg: "), "{said}");
    assert_eq!(unkept, None);
    assert_eq!(cache.held().1, 0);
    await_reaped(&service);
    assert_eq!(service.processes.started_tools(), 2);
}

#[test]
fn more_sound_than_the_length_says_is_an_error_and_nothing_is_kept() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; sound past the bucket bound was not read");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Six hours of silence are more buckets than a file of peaks may hold;
    // the asset says one hour, as a container with a wrong length would.
    let path = dir.path().join("longer.flac");
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=8000:cl=mono",
            "-t",
            "21600",
        ],
    );
    let mut asset = probed(&path);
    assert!(buckets_in(asset.duration) > MAX_PEAK_BUCKETS as u64);
    asset.duration = Time::from_seconds(3600.0);
    let cache = PeakCache::at(dir.path().join("peaks"));
    let service = SoundService::new(|| {}).unwrap();
    let request = service
        .peaks(PeakJob {
            project: Uuid::from_u128(1),
            asset: asset.clone(),
            cache: cache.clone(),
        })
        .unwrap();
    let (result, unkept) = await_peaked(&service, request);
    assert_eq!(
        result.unwrap_err(),
        "Media has more sound than its length says: too much for a waveform"
    );
    assert_eq!(unkept, None);
    // Nothing was cut short and kept as whole, and the process is gone.
    assert_eq!(cache.held().1, 0);
    assert_eq!(cache.load(&asset), KeptPeaks::Missing);
    await_reaped(&service);
    assert!(service.poll().is_none());
    assert_eq!(service.processes.started_tools(), 1);
}

// ---- Loudness ------------------------------------------------------------------

/// FFmpeg 8.1.2's report of a 3 s stereo tone at -20 dBFS, as it wrote it
/// with `-loglevel info -nostats`: the summary and the two lines after it.
const REPORT: &str = "\
[aist#0:0/pcm_s16le @ 0x792c1c180] Guessed Channel Layout: stereo
Input #0, wav, from 'tone.wav':
  Metadata:
    encoder         : Lavf62.12.102
  Duration: 00:00:05.00, bitrate: 1536 kb/s
  Stream #0:0: Audio: pcm_s16le ([1][0][0][0] / 0x0001), 48000 Hz, stereo, s16, 1536 kb/s
Stream mapping:
  Stream #0:0 -> #0:0 (pcm_s16le (native) -> pcm_s16le (native))
Output #0, null, to 'pipe:':
  Metadata:
    encoder         : Lavf62.12.102
  Stream #0:0: Audio: pcm_s16le, 48000 Hz, stereo, s16, 1536 kb/s
    Metadata:
      encoder         : Lavc62.28.102 pcm_s16le
[Parsed_ebur128_0 @ 0x792c0cfc0] Summary:

  Integrated loudness:
    I:         -20.0 LUFS
    Threshold: -30.0 LUFS

  Loudness range:
    LRA:        20.0 LU
    Threshold: -40.0 LUFS
    LRA low:   -40.0 LUFS
    LRA high:  -20.0 LUFS

  True peak:
    Peak:      -20.0 dBFS
[out#0/null @ 0x792c0c840] video:0KiB audio:562KiB subtitle:0KiB other streams:0KiB global headers:0KiB muxing overhead: unknown
size=N/A time=00:00:03.00 bitrate=N/A speed= 205x elapsed=0:00:00.01    
";

/// The report with other figures in its summary.
fn reported(integrated: &str, peak: &str) -> String {
    (REPORT.replace("I:         -20.0", &format!("I:         {integrated}")))
        .replace("Peak:      -20.0", &format!("Peak:      {peak}"))
}

#[test]
fn the_figures_are_read_from_the_summary_whatever_follows_or_comes_before() {
    let measured = |integrated, true_peak| {
        Ok(Loudness::Measured {
            integrated,
            true_peak,
        })
    };
    // The summary, and the two lines FFmpeg writes after it.
    assert_eq!(summary(REPORT), measured(-20.0, -20.0));
    assert_eq!(summary(&reported("-8.3", "0.4")), measured(-8.3, 0.4));
    // Warnings before it, in its own words and the decoder's.
    let warned = format!(
        "[mp3 @ 0x1] Header missing\n[Parsed_ebur128_0 @ 0x2] I: -5.0 LUFS seen early\n\
         Error while decoding stream #0:0: Invalid data found\n{REPORT}"
    );
    assert_eq!(summary(&warned), measured(-20.0, -20.0));
    // Two summaries (two meters in one graph): the last is the one.
    let twice = format!("{}{REPORT}", reported("-31.0", "-30.0"));
    assert_eq!(summary(&twice), measured(-20.0, -20.0));
    // Silence has no peak; a span too short for one block reads as the
    // floor with a peak, which is how the two are told apart.
    assert_eq!(summary(&reported("-70.0", "-inf")), Ok(Loudness::Silent));
    assert_eq!(summary(&reported("-70.0", "-38.1")), Ok(Loudness::TooShort));
    assert_eq!(summary(&reported("-69.9", "-38.1")), measured(-69.9, -38.1));
    assert_eq!(LOUDNESS_FLOOR, -70.0);
}

#[test]
fn a_report_cut_to_the_kept_tail_still_has_its_summary() {
    // Lines of warnings before the summary, far more than the tail keeps:
    // the tail starts in the middle of one of them.
    let noisy = format!(
        "{}{REPORT}",
        "[aac @ 0x7f8] Too many bits 8191 > 6144 per frame requested, clamping to max\n"
            .repeat(200)
    );
    let bytes = noisy.as_bytes();
    let tail = &bytes[bytes.len() - crate::process::STDERR_TAIL..];
    assert!(!tail.starts_with(b"[aac"));
    let tail = String::from_utf8_lossy(tail);
    assert_eq!(
        summary(&tail),
        Ok(Loudness::Measured {
            integrated: -20.0,
            true_peak: -20.0
        })
    );
    // A cut through a character is no matter.
    let mut wide = "é".repeat(5000).into_bytes();
    wide.extend_from_slice(REPORT.as_bytes());
    let tail = String::from_utf8_lossy(&wide[wide.len() - crate::process::STDERR_TAIL + 1..]);
    assert!(matches!(summary(&tail), Ok(Loudness::Measured { .. })));
}

#[test]
fn a_report_without_its_figures_is_an_error_in_words() {
    let said = |report: &str| summary(report).unwrap_err();
    assert_eq!(said(""), "its report has no loudness summary");
    let header = REPORT.find("[Parsed_ebur128_0").unwrap();
    assert_eq!(
        said(&REPORT[..header]),
        "its report has no loudness summary"
    );
    // Cut after the summary's first line: the tail held too little.
    let cut = REPORT.find("  Integrated").unwrap();
    assert_eq!(
        said(&REPORT[..cut]),
        "its loudness summary has no integrated loudness"
    );
    // The meter without `peak=true` has no true peak.
    let without = REPORT.replace("    Peak:      -20.0 dBFS\n", "");
    assert_eq!(said(&without), "its loudness summary has no true peak");
    // Figures that are not numbers, or in another unit.
    assert_eq!(
        said(&reported("nan", "-20.0")),
        "its loudness summary has no integrated loudness"
    );
    assert_eq!(
        said(&REPORT.replace(
            "-20.0 LUFS\n    Threshold: -30.0",
            "-20.0 dB\n    Threshold: -30.0"
        )),
        "its loudness summary has no integrated loudness"
    );
    assert_eq!(
        said(&reported("-20.0", "inf")),
        "its loudness summary has no true peak"
    );
}

fn audio(kind: EffectKind, params: &[(ParamKey, f32)]) -> Effect {
    Effect {
        params: (params.iter())
            .map(|&(key, value)| ParamValue { key, value })
            .collect(),
        ..Effect::new(kind)
    }
}

#[test]
fn a_measurement_hears_the_audio_effects_the_voice_would_and_the_meter_last() {
    let all = Filters::default();
    // The meter always hears two channels, summed as a voice's are.
    let meter = "aformat=channel_layouts=stereo,ebur128=peak=true:framelog=verbose";
    assert_eq!(graph(&[], &all), meter);
    // A band, then a disabled limiter, a pitch and a picture effect, none
    // of which is a filter here, then a high-pass: stack order, at 48 kHz.
    let band = audio(
        EffectKind::EqBand,
        &[(ParamKey::Frequency, 1000.0), (ParamKey::Gain, -12.0)],
    );
    let off = Effect {
        enabled: false,
        ..Effect::new(EffectKind::Limiter)
    };
    let stack = [
        band.clone(),
        off,
        Effect::new(EffectKind::Pitch),
        Effect::new(EffectKind::Blur),
        Effect::new(EffectKind::HighPass),
    ];
    assert_eq!(
        graph(&stack, &all),
        format!(
            "aresample=48000,equalizer=f=1000.000:t=o:w=1.000:g=-12.000,highpass=f=80.000,{meter}"
        )
    );
    // A stack with nothing heard is the sum and the meter alone.
    assert_eq!(graph(&stack[1..4], &all), meter);
    // A kind this FFmpeg does not run is left out, as from a voice.
    let some = Filters::without(&[EffectKind::EqBand]);
    assert_eq!(
        graph(&stack, &some),
        format!("aresample=48000,highpass=f=80.000,{meter}")
    );
    // Centre reduction is kept: the measurement is of two channels.
    assert!(graph(&[Effect::new(EffectKind::CenterCut)], &all).contains("pan=stereo"));
}

#[test]
fn a_span_that_cannot_be_measured_is_refused_before_any_process() {
    let service = SoundService::new(|| {}).unwrap();
    let sound = Asset::new("take.wav".into(), MediaKind::Audio, Time::from_seconds(4.0));
    let job = |asset: &Asset, from: f64, span: f64| LoudnessJob {
        project: Uuid::from_u128(1),
        asset: asset.clone(),
        from: Time::from_seconds(from),
        span: Time::from_seconds(span),
        effects: Vec::new(),
    };
    let refused = |job: LoudnessJob, why: &str| match service.loudness(job) {
        Err(MediaError::Invalid(said)) => assert_eq!(said, why),
        other => panic!("{other:?}"),
    };
    let mut silent = Asset::new(
        "silent.mp4".into(),
        MediaKind::Video,
        Time::from_seconds(4.0),
    );
    (silent.width, silent.height) = (320, 240);
    refused(job(&silent, 0.0, 1.0), "Media has no sound to measure");
    let none = Asset::new("none.wav".into(), MediaKind::Audio, Time::ZERO);
    refused(job(&none, 0.0, 1.0), "Media has no length");
    refused(job(&sound, 0.0, 0.0), "The span to measure is empty");
    refused(job(&sound, -1.0, 1.0), "The span to measure is empty");
    refused(
        job(&sound, 4.0, 1.0),
        "The span to measure starts after the media ends",
    );
    let mut many = job(&sound, 0.0, 1.0);
    many.effects = vec![Effect::new(EffectKind::HighPass); 17];
    refused(many, "A clip has at most 16 effects");
    assert_eq!(service.processes.started_tools(), 0);
    assert!(!service.busy());
    assert!(service.poll().is_none());
}

/// A 1 kHz sine in two channels, `decibels` under full scale at its peak,
/// 16-bit at 48 kHz.
fn stereo_tone(dir: &Path, decibels: f64, seconds: f64) -> Asset {
    let path = dir.join(format!("tone{decibels}x{seconds}.wav"));
    let amplitude = 10_f64.powf(decibels / 20.0);
    let wave = format!("{amplitude}*sin(2*PI*1000*t)");
    let source = format!("aevalsrc={wave}|{wave}:s=48000:d={seconds}");
    made(&path, &["-f", "lavfi", "-i", &source, "-c:a", "pcm_s16le"]);
    probed(&path)
}

/// What the lane says of a span of `asset`, and the progress it said on the
/// way, which never goes back or past the end.
fn measured(
    service: &SoundService,
    asset: &Asset,
    from: f64,
    span: f64,
    effects: Vec<Effect>,
) -> Result<Loudness, String> {
    let request = service
        .loudness(LoudnessJob {
            project: Uuid::from_u128(1),
            asset: asset.clone(),
            from: Time::from_seconds(from),
            span: Time::from_seconds(span),
            effects,
        })
        .unwrap();
    let limit = Instant::now() + Duration::from_secs(60);
    let mut said = 0.0;
    loop {
        match service.poll() {
            Some(SoundEvent::Progress { request: of, done }) if of == request => {
                assert!(done >= said && done <= 1.0, "{said} then {done}");
                said = done;
            }
            Some(SoundEvent::Measured {
                request: of,
                project,
                asset_id,
                source,
                result,
            }) if of == request => {
                assert_eq!((project, asset_id), (Uuid::from_u128(1), asset.id));
                assert_eq!(source, SourceIdentity::of(asset));
                return result;
            }
            Some(other) => panic!("{other:?}"),
            None => {
                assert!(
                    Instant::now() < limit,
                    "timed out waiting for the measurement"
                );
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn integrated(result: Result<Loudness, String>) -> (f32, f32) {
    match result {
        Ok(Loudness::Measured {
            integrated,
            true_peak,
        }) => (integrated, true_peak),
        other => panic!("{other:?}"),
    }
}

#[test]
fn tones_silence_and_a_short_tone_measure_as_they_are() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the loudness fixtures were not measured");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    // A 1 kHz sine at full scale in one channel reads -3.01 LUFS (ITU-R
    // BS.1770); in two it is 3.01 dB more, so at -20 dBFS in both, -20.
    let tone = stereo_tone(dir.path(), -20.0, 5.0);
    let (loud, peak) = integrated(measured(&service, &tone, 0.0, 5.0, Vec::new()));
    eprintln!("MEASURE: stereo 1 kHz at -20 dBFS: {loud} LUFS, true peak {peak} dBFS");
    assert!((loud + 20.0).abs() <= 0.5, "{loud}");
    assert!((peak + 20.0).abs() <= 0.5, "{peak}");
    // 6 dB quieter is 6 LU lower.
    let quieter = stereo_tone(dir.path(), -26.0, 5.0);
    let (lower, _) = integrated(measured(&service, &quieter, 0.0, 5.0, Vec::new()));
    eprintln!("MEASURE: 6 dB quieter: {lower} LUFS");
    assert!((loud - lower - 6.0).abs() <= 0.2, "{loud} and {lower}");
    // A span of it, from inside: the same tone, the same loudness.
    let (span, _) = integrated(measured(&service, &tone, 1.5, 2.0, Vec::new()));
    assert!((span - loud).abs() <= 0.2, "{span} and {loud}");

    // Through a band of -12 dB at its frequency, about 12 LU lower.
    let band = audio(
        EffectKind::EqBand,
        &[(ParamKey::Frequency, 1000.0), (ParamKey::Gain, -12.0)],
    );
    let (banded, _) = integrated(measured(&service, &tone, 0.0, 5.0, vec![band]));
    eprintln!("MEASURE: through a -12 dB band at 1 kHz: {banded} LUFS");
    assert!((loud - banded - 12.0).abs() <= 0.5, "{loud} and {banded}");

    // Silence has no peak.
    let path = dir.path().join("silence.wav");
    made(
        &path,
        &["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-t", "2"],
    );
    let silence = probed(&path);
    assert_eq!(
        measured(&service, &silence, 0.0, 2.0, Vec::new()),
        Ok(Loudness::Silent)
    );
    // 0.3 s of the tone is under one 0.4 s block: the floor, with a peak.
    let short = stereo_tone(dir.path(), -20.0, 0.3);
    assert_eq!(
        measured(&service, &short, 0.0, 0.3, Vec::new()),
        Ok(Loudness::TooShort)
    );
    // And a span of the long tone that short.
    assert_eq!(
        measured(&service, &tone, 2.0, 0.3, Vec::new()),
        Ok(Loudness::TooShort)
    );
    await_reaped(&service);
}

#[test]
fn a_source_of_other_channels_measures_as_a_stereo_output_plays_it() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the channel fixtures were not measured");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    // 5.1 with the tone at -6 dBFS in left, right and centre. A voice sums
    // it to two channels, centre at -3 dB into each and then scaled to
    // hold the sum under full scale: -1.4 dBFS, against -6.0 in its own
    // layout (on this host the voice's own output read -1.3 LUFS and -1.4
    // dBFS through the same meter).
    let path = dir.path().join("six.wav");
    let wave = format!("{}*sin(2*PI*1000*t)", 10_f64.powf(-6.0 / 20.0));
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("aevalsrc={wave}|{wave}|{wave}|0|0|0:s=48000:d=3:c=5.1"),
            "-c:a",
            "pcm_s16le",
        ],
    );
    let six = probed(&path);
    let (loud, peak) = integrated(measured(&service, &six, 0.0, 3.0, Vec::new()));
    eprintln!("MEASURE: 5.1 at -6 dBFS in L, R and C: {loud} LUFS, true peak {peak} dBFS");
    assert!((peak + 1.4).abs() <= 0.3, "{peak}");
    assert!((loud + 1.3).abs() <= 0.5, "{loud}");
    // One channel at -20 dBFS is spread over two at -23: its loudness is
    // the same either way, its peak 3 dB lower than the file's.
    let path = dir.path().join("mono.wav");
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.1*sin(2*PI*1000*t):s=48000:d=3",
            "-c:a",
            "pcm_s16le",
        ],
    );
    let mono = probed(&path);
    let (loud, peak) = integrated(measured(&service, &mono, 0.0, 3.0, Vec::new()));
    eprintln!("MEASURE: mono at -20 dBFS: {loud} LUFS, true peak {peak} dBFS");
    assert!((loud + 23.0).abs() <= 0.5, "{loud}");
    assert!((peak + 23.0).abs() <= 0.3, "{peak}");
    await_reaped(&service);
}

#[test]
fn a_span_reads_its_own_part_of_the_file() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the span fixture was not measured");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Two seconds of silence, then two of the tone at -20 dBFS.
    let path = dir.path().join("after.wav");
    // A comma inside a filter's options is escaped.
    let wave = r"0.1*sin(2*PI*1000*t)*gte(t\,2)";
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("aevalsrc={wave}|{wave}:s=48000:d=4"),
            "-c:a",
            "pcm_s16le",
        ],
    );
    let asset = probed(&path);
    let service = SoundService::new(|| {}).unwrap();
    assert_eq!(
        measured(&service, &asset, 0.0, 2.0, Vec::new()),
        Ok(Loudness::Silent)
    );
    let (loud, _) = integrated(measured(&service, &asset, 2.0, 2.0, Vec::new()));
    assert!((loud + 20.0).abs() <= 0.5, "{loud}");
    // A span past the end is what of it the file has.
    let (rest, _) = integrated(measured(&service, &asset, 2.5, 10.0, Vec::new()));
    assert!((rest + 20.0).abs() <= 0.5, "{rest}");
    await_reaped(&service);
}

#[test]
fn a_file_that_is_no_sound_is_an_answer_in_ffmpegs_words() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; a damaged file was not measured");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    let noise = sound_at(&dir.path().join("noise.wav"), 4096);
    let said = measured(&service, &noise, 0.0, 1.0, Vec::new()).unwrap_err();
    assert!(
        said.starts_with("ffmpeg: could not measure the sound"),
        "{said}"
    );
    let gone = Asset::new(
        dir.path().join("gone.wav"),
        MediaKind::Audio,
        Time::from_seconds(1.0),
    );
    let said = measured(&service, &gone, 0.0, 1.0, Vec::new()).unwrap_err();
    assert!(said.starts_with("ffmpeg: "), "{said}");
    await_reaped(&service);
    assert_eq!(service.processes.started_tools(), 2);
}

#[test]
fn a_measurement_given_up_or_replaced_is_reaped_and_says_nothing() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; cancelling a measurement was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let asset = long(dir.path());
    let service = SoundService::new(|| {}).unwrap();
    let job = |project: u128| LoudnessJob {
        project: Uuid::from_u128(project),
        asset: asset.clone(),
        from: Time::ZERO,
        span: asset.duration,
        effects: Vec::new(),
    };
    // The meter's true peak makes even 8 kHz slow to measure (about 16 s
    // for these two hours), so each is caught at its first word; that a
    // measurement ends is proved elsewhere.
    // Given up by its name.
    let asked = service.loudness(job(1)).unwrap();
    await_progress(&service, asked);
    service.cancel(asked);
    await_reaped(&service);
    assert!(service.poll().is_none());
    assert_eq!(service.processes.started_tools(), 1);

    // Replaced: the first is stopped and says nothing more, and what is
    // said after is the second's. Then every request is given up at once,
    // as when the project is replaced.
    let first = service.loudness(job(2)).unwrap();
    await_progress(&service, first);
    let second = service.loudness(job(3)).unwrap();
    await_progress(&service, second);
    service.cancel_all();
    await_reaped(&service);
    assert!(service.poll().is_none());
    assert_eq!(service.processes.started_tools(), 3);
}

// ---- Beats ---------------------------------------------------------------------

/// A little noise that is the same on every run.
struct Noise(u64);

impl Noise {
    /// A whole number from 0 to `below`, less one.
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }
}

/// The peaks of a click track at `bpm`, the first click at `first` seconds,
/// over `seconds`, and when each click is. A bed of quiet noise around
/// -80 dBFS; a click rises from it to near full scale in one bucket, with a
/// low part as a kick drum has, and dies away over four more.
fn clicks(bpm: f64, first: f64, seconds: f64) -> (Peaks, Vec<f64>) {
    let mut noise = Noise(0x9e37_79b9_7f4a_7c15);
    let count = (seconds * f64::from(PEAKS_PER_SECOND)).round() as usize;
    let mut buckets: Vec<[u8; 4]> = (0..count)
        .map(|_| {
            let rms = 42 + noise.next(5) as u8;
            [(-1i8) as u8, 1, rms, rms - 8 + noise.next(5) as u8]
        })
        .collect();
    let mut times = Vec::new();
    let mut at = first;
    while at < seconds {
        let start = (at * f64::from(PEAKS_PER_SECOND)).floor() as usize;
        for (step, level) in [235u8, 205, 165, 120, 80].into_iter().enumerate() {
            if let Some(bucket) = buckets.get_mut(start + step) {
                *bucket = [(-110i8) as u8, 110, level, level - 6];
            }
        }
        times.push(at);
        at = first + times.len() as f64 * 60.0 / bpm;
    }
    (Peaks { buckets }, times)
}

/// Whether a grid keeps the tempo within 1 BPM, has a beat within 20 ms of
/// every click and no beat farther than that from one.
fn on_the_clicks(grid: &BeatGrid, clicks: &[f64], bpm: f64) {
    assert!(
        (f64::from(grid.bpm) - bpm).abs() <= 1.0,
        "{} BPM for {bpm}",
        grid.bpm
    );
    assert!(grid.beats.is_sorted());
    let off = |from: &[f64], at: f64| {
        (from.iter())
            .map(|time| (time - at).abs())
            .fold(f64::INFINITY, f64::min)
    };
    let beats: Vec<f64> = grid
        .beats
        .iter()
        .map(|&ms| f64::from(ms) / 1000.0)
        .collect();
    let mut worst = 0.0f64;
    for click in clicks {
        let miss = off(&beats, *click);
        worst = worst.max(miss);
        assert!(
            miss <= 0.020,
            "no beat within 20 ms of the click at {click} s"
        );
    }
    for beat in &beats {
        assert!(
            off(clicks, *beat) <= 0.020,
            "a beat at {beat} s is on no click"
        );
    }
    assert_eq!(beats.len(), clicks.len());
    eprintln!(
        "MEASURE: {bpm} BPM clicks: {} BPM, {} beats, farthest {:.1} ms from its click",
        grid.bpm,
        beats.len(),
        worst * 1000.0
    );
}

#[test]
fn a_click_track_keeps_its_tempo_and_its_beats_over_three_minutes() {
    assert_eq!(MAX_BEATS, 16_384);
    // 60 BPM is the slowest period looked for.
    for bpm in [60.0, 90.0, 120.0, 150.0] {
        let (peaks, times) = clicks(bpm, 0.5, 180.0);
        let grid = beats(&peaks).unwrap_or_else(|| panic!("no beats at {bpm} BPM"));
        on_the_clicks(&grid, &times, bpm);
    }
    // Off the bucket grid too: 128 BPM is a click every 468.75 ms.
    let (peaks, times) = clicks(128.0, 0.5, 180.0);
    on_the_clicks(&beats(&peaks).unwrap(), &times, 128.0);
}

#[test]
fn a_fast_click_track_reads_at_its_tempo_or_half_of_it() {
    // Above about 170 BPM a period is a third of a bucket or more off the
    // grid, and the prior on 120 BPM may prefer every other click; it must
    // never prefer every third.
    for bpm in [170.0, 180.0, 190.0, 200.0] {
        let (peaks, times) = clicks(bpm, 0.5, 180.0);
        let grid = beats(&peaks).unwrap_or_else(|| panic!("no beats at {bpm} BPM"));
        assert!((60.0..=200.0).contains(&grid.bpm), "{} BPM", grid.bpm);
        if (f64::from(grid.bpm) - bpm).abs() <= 1.0 {
            on_the_clicks(&grid, &times, bpm);
            continue;
        }
        // Half: every other click has a beat, and every beat is on one.
        let half: Vec<f64> = (times.iter().step_by(2).copied()).collect();
        let other: Vec<f64> = (times.iter().skip(1).step_by(2).copied()).collect();
        let first = f64::from(grid.beats[0]) / 1000.0;
        let on = if (first - half[0]).abs() <= 0.020 {
            half
        } else {
            other
        };
        on_the_clicks(&grid, &on, bpm / 2.0);
    }
}

#[test]
fn a_click_track_that_starts_late_is_found_in_its_phase() {
    // A first click a little over a third of a beat in, and one in the
    // second beat's place: the grid is on the clicks, not on 0.
    for first in [0.37, 0.71, 2.13] {
        let (peaks, times) = clicks(120.0, first, 60.0);
        let grid = beats(&peaks).unwrap();
        on_the_clicks(&grid, &times, 120.0);
        let start = f64::from(grid.beats[0]) / 1000.0;
        assert!((start - first).abs() <= 0.020, "{start} for {first}");
    }
    // A file that starts on its beat, as a loop does: the first click is in
    // the file's first bucket, and so is its beat.
    let (peaks, times) = clicks(120.0, 0.0, 60.0);
    let grid = beats(&peaks).unwrap();
    on_the_clicks(&grid, &times, 120.0);
    assert_eq!(grid.beats[0], 5);
    // Silence before the first click and after the last has no beats.
    let (mut peaks, times) = clicks(120.0, 10.0, 50.0);
    peaks.buckets.truncate(4000);
    peaks.buckets.extend(vec![[0; 4]; 2000]);
    let times: Vec<f64> = times.into_iter().filter(|time| *time < 39.9).collect();
    on_the_clicks(&beats(&peaks).unwrap(), &times, 120.0);
}

#[test]
fn silence_steady_sound_noise_and_a_short_file_have_no_beats() {
    let peaks = |buckets: Vec<[u8; 4]>| Peaks { buckets };
    assert_eq!(beats(&Peaks::default()), None);
    assert_eq!(beats(&peaks(vec![[0; 4]; 18_000])), None);
    // A tone that does not change.
    assert_eq!(
        beats(&peaks(vec![[(-64i8) as u8, 64, 231, 200]; 18_000])),
        None
    );
    // Steady noise, loud and quiet, a little and a lot of it, three minutes
    // and ten seconds.
    for (seed, level, swing, length) in [
        (1, 200, 6, 18_000),
        (2, 120, 30, 18_000),
        (3, 60, 60, 18_000),
        (4, 160, 20, 1_000),
        (5, 220, 35, 500),
    ] {
        let mut noise = Noise(0x2545_f491_4f6c_dd1d ^ seed);
        let buckets = (0..length)
            .map(|_| {
                let rms = level - swing / 2 + noise.next(u64::from(swing) + 1) as u8;
                let low = rms - swing / 2 + noise.next(u64::from(swing) + 1) as u8;
                [(-90i8) as u8, 90, rms, low]
            })
            .collect();
        assert_eq!(beats(&peaks(buckets)), None, "noise {seed}");
    }
    // Clicks, but under four periods of them: a second at 200 BPM.
    let (short, _) = clicks(200.0, 0.1, 1.1);
    assert_eq!(beats(&short), None);
}

#[test]
fn the_longest_file_gives_at_most_the_bound_of_beats() {
    let _heavy = crate::heavy_test();
    // 5.8 hours of clicks at 120 BPM are 41,943 beats.
    let (peaks, times) = clicks(120.0, 0.5, MAX_PEAK_BUCKETS as f64 / 100.0);
    assert_eq!(peaks.buckets.len(), MAX_PEAK_BUCKETS);
    let began = Instant::now();
    let grid = beats(&peaks).unwrap();
    eprintln!(
        "MEASURE: beats of {} buckets found in {:.2} s",
        peaks.buckets.len(),
        began.elapsed().as_secs_f64()
    );
    assert_eq!(grid.beats.len(), MAX_BEATS);
    // They are the first beats, and on their clicks.
    on_the_clicks(
        &BeatGrid {
            beats: grid.beats.clone(),
            ..grid
        },
        &times[..MAX_BEATS],
        120.0,
    );
}

/// What the lane says of a file's beats, waited for.
fn await_beats(service: &SoundService, request: u64) -> Result<Option<BeatGrid>, String> {
    let limit = Instant::now() + Duration::from_secs(60);
    loop {
        match service.poll() {
            Some(SoundEvent::Beats {
                request: of,
                result,
                ..
            }) if of == request => return result,
            Some(SoundEvent::Beats { request: of, .. }) => panic!("{of} answered for {request}"),
            Some(_) => {}
            None => {
                assert!(Instant::now() < limit, "timed out waiting for the beats");
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

#[test]
fn beats_are_found_in_kept_peaks_and_leave_the_files_reading_alone() {
    let lane = Gated::new();
    let (kept, times) = clicks(120.0, 0.25, 30.0);
    (lane.cache)
        .store(&lane.asset, &Stamp::of(&lane.asset), &kept)
        .unwrap();
    // The file's sound is being read, and waits at its gate.
    let reading = lane.service.peaks(lane.job(1)).unwrap();
    lane.started(1);
    // Its beats are asked for meanwhile: they are found in what is kept and
    // answered while the reading waits, which goes on.
    let asked = lane.service.beats(lane.job(2)).unwrap();
    let mut answer = None;
    lane.wait("the beats", || {
        while let Some(event) = lane.service.poll() {
            if let SoundEvent::Beats {
                request,
                project,
                asset_id,
                source,
                result,
            } = event
            {
                assert_eq!((request, project), (asked, Uuid::from_u128(2)));
                assert_eq!(asset_id, lane.asset.id);
                assert_eq!(source, SourceIdentity::of(&lane.asset));
                answer = Some(result);
            }
        }
        answer.is_some()
    });
    on_the_clicks(&answer.unwrap().unwrap().unwrap(), &times, 120.0);
    assert_eq!(lane.started.load(Ordering::Acquire), 1);
    lane.open();
    assert_eq!(lane.answered(), vec![reading]);

    // The other way round: the file's sound read again while its beats are
    // being found stops neither, and kept peaks asked for wait their turn.
    lane.hold_beats(true);
    *lane.gate.0.lock().unwrap() = false;
    // The gated reading keeps other peaks: the clicks go back in.
    (lane.cache)
        .store(&lane.asset, &Stamp::of(&lane.asset), &kept)
        .unwrap();
    let finding = lane.service.beats(lane.job(3)).unwrap();
    lane.wait("the beats to begin", || {
        lane.finding.load(Ordering::Acquire) == 2
    });
    let loading = lane.service.load_peaks(lane.job(3)).unwrap();
    let reading = lane.service.peaks(lane.job(3)).unwrap();
    lane.started(2);
    // The beats are let go first: the stand-in reading keeps other peaks
    // when it ends, which have none.
    lane.hold_beats(false);
    lane.wait("the beats to end", || {
        lane.found.load(Ordering::Acquire) == 2
    });
    lane.open();
    let order: Vec<(u64, &str)> = (lane.events().into_iter())
        .filter_map(|event| match event {
            SoundEvent::Beats {
                request,
                result: Ok(Some(_)),
                ..
            } => Some((request, "beats")),
            SoundEvent::Loaded { request, .. } => Some((request, "loaded")),
            SoundEvent::Peaked { request, .. } => Some((request, "peaked")),
            _ => None,
        })
        .collect();
    let mut order = order;
    order.sort();
    assert_eq!(
        order,
        vec![(finding, "beats"), (loading, "loaded"), (reading, "peaked")]
    );
}

#[test]
fn beats_given_up_or_replaced_answer_nothing() {
    let lane = Gated::new();
    let (kept, _) = clicks(120.0, 0.25, 30.0);
    (lane.cache)
        .store(&lane.asset, &Stamp::of(&lane.asset), &kept)
        .unwrap();
    let other = lane.another(16);
    (lane.cache)
        .store(&other.asset, &Stamp::of(&other.asset), &kept)
        .unwrap();
    lane.hold_beats(true);
    // Under way, then replaced by a newer for the same media; that newer is
    // replaced while it waits by a third. Another media's waits, and is
    // given up by its name before it begins.
    let first = lane.service.beats(lane.job(1)).unwrap();
    lane.wait("the first to begin", || {
        lane.finding.load(Ordering::Acquire) == 1
    });
    let second = lane.service.beats(lane.job(2)).unwrap();
    let elsewhere = lane.service.beats(other.clone()).unwrap();
    let third = lane.service.beats(lane.job(3)).unwrap();
    lane.service.cancel(elsewhere);
    assert!(first < second && second < third);
    lane.hold_beats(false);
    lane.wait("the third", || lane.found.load(Ordering::Acquire) == 2);
    thread::sleep(Duration::from_millis(20));
    let answers: Vec<(u64, Uuid)> = (lane.events().into_iter())
        .map(|event| match event {
            SoundEvent::Beats {
                request,
                project,
                result: Ok(Some(_)),
                ..
            } => (request, project),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(answers, vec![(third, Uuid::from_u128(3))]);
    // The first was begun and ended, unheard; the second and the other
    // media's never began.
    assert_eq!(lane.finding.load(Ordering::Acquire), 2);

    // Given up by its name while under way.
    lane.hold_beats(true);
    let stopped = lane.service.beats(lane.job(4)).unwrap();
    lane.wait("it to begin", || lane.finding.load(Ordering::Acquire) == 3);
    lane.service.cancel(stopped);
    assert!(!lane.service.busy());
    lane.hold_beats(false);
    lane.wait("it to end", || lane.found.load(Ordering::Acquire) == 3);
    thread::sleep(Duration::from_millis(20));
    assert!(lane.events().is_empty());

    // Another project: every request is given up at once, under way or not.
    lane.hold_beats(true);
    lane.service.beats(lane.job(5)).unwrap();
    lane.wait("it to begin", || lane.finding.load(Ordering::Acquire) == 4);
    lane.service.beats(other).unwrap();
    lane.service.load_peaks(lane.job(5)).unwrap();
    lane.service.cancel_all();
    lane.hold_beats(false);
    lane.wait("it to end", || lane.found.load(Ordering::Acquire) == 4);
    thread::sleep(Duration::from_millis(20));
    assert!(lane.events().is_empty());
    assert_eq!(lane.finding.load(Ordering::Acquire), 4);
    assert_eq!(lane.started.load(Ordering::Acquire), 0);
}

#[test]
fn beats_wait_in_the_queue_of_kept_peaks_and_the_seventeenth_is_busy() {
    let lane = Gated::new();
    lane.hold_beats(true);
    let under_way = lane.service.beats(lane.job(1)).unwrap();
    lane.wait("it to begin", || lane.finding.load(Ordering::Acquire) == 1);
    // Sixteen wait: beats and kept peaks of fifteen other files, and the
    // kept peaks of the file whose beats are being found, which is another
    // target and replaces nothing.
    let mut waiting = vec![lane.service.load_peaks(lane.job(1)).unwrap()];
    let others: Vec<PeakJob> = (0..15).map(|index| lane.another(index + 1)).collect();
    for (index, job) in others.iter().enumerate() {
        waiting.push(if index % 2 == 0 {
            lane.service.beats(job.clone()).unwrap()
        } else {
            lane.service.load_peaks(job.clone()).unwrap()
        });
    }
    assert!(matches!(
        lane.service.beats(lane.another(100)),
        Err(MediaError::Busy)
    ));
    assert!(matches!(
        lane.service.load_peaks(lane.another(100)),
        Err(MediaError::Busy)
    ));
    // A newer for beats that wait takes their place and needs no room.
    let newer = lane.service.beats(others[0].clone()).unwrap();
    // One for the beats under way waits, so it needs room, and without it
    // the one under way is not given up for nothing.
    assert!(matches!(
        lane.service.beats(lane.job(2)),
        Err(MediaError::Busy)
    ));
    lane.hold_beats(false);
    let answered: Vec<u64> = (lane.events().into_iter())
        .map(|event| match event {
            SoundEvent::Beats { request, .. } | SoundEvent::Loaded { request, .. } => request,
            other => panic!("{other:?}"),
        })
        .collect();
    let mut expected: Vec<u64> = std::iter::once(under_way)
        .chain(
            waiting
                .iter()
                .copied()
                .filter(|request| *request != waiting[1]),
        )
        .collect();
    expected.push(newer);
    assert_eq!(answered, expected);
}

#[test]
fn beats_of_peaks_not_kept_or_of_media_that_changed_are_an_error_in_words() {
    let lane = Gated::new();
    let said = |request: u64| await_beats(&lane.service, request).unwrap_err();
    assert_eq!(
        said(lane.service.beats(lane.job(1)).unwrap()),
        "The media's sound has not been read: its peaks are needed to find its beats"
    );
    let (kept, _) = clicks(120.0, 0.25, 30.0);
    (lane.cache)
        .store(&lane.asset, &Stamp::of(&lane.asset), &kept)
        .unwrap();
    assert!(matches!(
        await_beats(&lane.service, lane.service.beats(lane.job(1)).unwrap()),
        Ok(Some(_))
    ));
    fs::write(&lane.asset.path, vec![7u8; 65]).unwrap();
    assert_eq!(
        said(lane.service.beats(lane.job(1)).unwrap()),
        "The media changed since its sound was read: its peaks are needed again to find its beats"
    );
    // Kept peaks of silence are no error: they have no beats.
    let quiet = lane.another(20);
    (lane.cache)
        .store(
            &quiet.asset,
            &Stamp::of(&quiet.asset),
            &Peaks {
                buckets: vec![[0; 4]; 3000],
            },
        )
        .unwrap();
    assert_eq!(
        await_beats(&lane.service, lane.service.beats(quiet).unwrap()),
        Ok(None)
    );
    assert_eq!(lane.started.load(Ordering::Acquire), 0);
}

#[test]
fn media_that_can_have_no_peaks_is_refused_its_beats_before_the_cache_is_read() {
    let lane = Gated::new();
    let refused = |asset: Asset, why: &str| match lane.service.beats(PeakJob {
        asset,
        ..lane.job(1)
    }) {
        Err(MediaError::Invalid(said)) => assert_eq!(said, why),
        other => panic!("{other:?}"),
    };
    let mut silent = Asset::new(
        "silent.mp4".into(),
        MediaKind::Video,
        Time::from_seconds(4.0),
    );
    (silent.width, silent.height) = (320, 240);
    refused(silent, "Media has no sound to find beats in");
    refused(
        Asset::new("none.wav".into(), MediaKind::Audio, Time::ZERO),
        "Media has no length",
    );
    let over = Asset::new(
        "day.wav".into(),
        MediaKind::Audio,
        Time(MAX_PEAK_BUCKETS as i64 * 10_000 + 1),
    );
    refused(
        over,
        "Media is too long to find beats in: the limit is 5.8 hours",
    );
    assert!(!lane.service.busy());
    assert!(lane.service.poll().is_none());
    assert_eq!(lane.finding.load(Ordering::Acquire), 0);
}

/// A click track of a kick and a tick at `bpm`, the first at `first`
/// seconds, `seconds` long, as FFmpeg's expression for `aevalsrc`, and when
/// each click is.
fn click_track(bpm: f64, first: f64, seconds: f64) -> (String, Vec<f64>) {
    let period = 60.0 / bpm;
    // Commas inside the expression are escaped.
    let phase = format!(r"mod(t-{first}\,{period})");
    let wave = format!(
        r"gte(t\,{first})*(0.7*sin(2*PI*55*{phase})*exp(-30*{phase})+0.3*sin(2*PI*1500*t)*lt({phase}\,0.01))"
    );
    let times = (0..)
        .map(|index| first + f64::from(index) * period)
        .take_while(|time| *time < seconds)
        .collect();
    (format!("aevalsrc={wave}:s=44100:d={seconds}"), times)
}

/// A file's beats through the lane: its peaks are read and kept, then its
/// beats are found in them.
fn beats_through_the_lane(asset: &Asset, dir: &Path) -> BeatGrid {
    let cache = PeakCache::at(dir.join("peaks"));
    read(asset, &cache);
    let service = SoundService::new(|| {}).unwrap();
    let request = service
        .beats(PeakJob {
            project: Uuid::from_u128(1),
            asset: asset.clone(),
            cache,
        })
        .unwrap();
    let grid = await_beats(&service, request)
        .unwrap()
        .expect("a clear beat");
    // Finding beats starts no process.
    assert_eq!(service.processes.started_tools(), 0);
    grid
}

#[test]
fn a_click_track_from_ffmpeg_has_its_beats_found_through_its_peaks() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the click track was not made");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // 128 BPM, off the bucket grid, from 0.3 s.
    let (source, times) = click_track(128.0, 0.3, 40.0);
    let path = dir.path().join("clicks.wav");
    made(&path, &["-f", "lavfi", "-i", &source, "-c:a", "pcm_s16le"]);
    let asset = probed(&path);
    on_the_clicks(&beats_through_the_lane(&asset, dir.path()), &times, 128.0);
}

#[test]
fn beats_of_sound_that_starts_late_in_its_file_are_in_container_time() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the late click track was not made");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Twelve seconds of picture, and ten of clicks at 100 BPM that begin a
    // second in: their first click is at 1.3 s of the file.
    let (source, times) = click_track(100.0, 0.3, 10.0);
    let path = dir.path().join("late.mp4");
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x48:r=30:d=12",
            "-itsoffset",
            "1",
            "-f",
            "lavfi",
            "-i",
            &source,
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ],
    );
    let asset = probed(&path);
    assert!(asset.kind == MediaKind::Video && asset.has_audio);
    let late: Vec<f64> = times.iter().map(|time| time + 1.0).collect();
    let grid = beats_through_the_lane(&asset, dir.path());
    on_the_clicks(&grid, &late, 100.0);
    assert!(grid.beats[0] >= 1280, "{:?}", &grid.beats[..3]);
}

// ---- Written files -------------------------------------------------------------

#[test]
fn a_written_file_and_the_same_medias_other_jobs_leave_each_other_alone() {
    let lane = Gated::new();
    let peaks = lane.service.peaks(lane.job(1)).unwrap();
    lane.started(1);
    // A file of the same media, and a measurement of the very span it
    // writes, while its peaks are read: both wait, and stop nothing.
    let saved = lane.service.extract(lane.save(1, 0.0, 1.0)).unwrap();
    assert_eq!(saved.replaced, Vec::<u64>::new());
    let saved = saved.request;
    let measured = lane.service.loudness(lane.span(1, 0.0, 1.0)).unwrap();
    // Another span of it is another file, and both are written.
    let other = lane.service.extract(lane.save(1, 0.5, 1.0)).unwrap();
    assert_eq!(other.replaced, Vec::<u64>::new());
    let other = other.request;
    // The same span again takes the place of the one that waits, by name.
    let mut again = lane.save(2, 0.0, 1.0);
    again.to = lane.dir.path().join("again.wav");
    let newer = lane.service.extract(again).unwrap();
    assert_eq!(newer.replaced, vec![saved]);
    let newer = newer.request;
    thread::sleep(Duration::from_millis(20));
    assert_eq!(lane.started.load(Ordering::Acquire), 1);
    lane.open();
    let events = lane.events();
    let answered: Vec<u64> = (events.iter())
        .filter_map(|event| match event {
            SoundEvent::Progress { .. } => None,
            other => Some(other.request()),
        })
        .collect();
    assert_eq!(answered, vec![peaks, measured, other, newer]);
    let Some(SoundEvent::Extracted {
        project,
        asset_id,
        source,
        to,
        result,
        ..
    }) = events.last()
    else {
        panic!("{events:?}");
    };
    assert_eq!((*project, *asset_id), (Uuid::from_u128(2), lane.asset.id));
    assert_eq!(*source, SourceIdentity::of(&lane.asset));
    assert_eq!(*to, lane.dir.path().join("again.wav"));
    assert_eq!(*result, Ok(Time::from_seconds(1.0)));

    // The other way round: peaks asked for while a file is written leave it
    // writing.
    *lane.gate.0.lock().unwrap() = false;
    let saved = lane
        .service
        .extract(lane.save(1, 0.0, 1.0))
        .unwrap()
        .request;
    lane.started(5);
    let peaks = lane.service.peaks(lane.job(1)).unwrap();
    lane.open();
    assert_eq!(lane.answered(), vec![saved, peaks]);
}

#[test]
fn a_file_being_written_outlives_a_project_replace_and_is_given_up_by_name() {
    let lane = Gated::new();
    let saved = lane
        .service
        .extract(lane.save(1, 0.0, 1.0))
        .unwrap()
        .request;
    lane.started(1);
    lane.wait("its progress", || lane.wakes.load(Ordering::Acquire) >= 2);
    let peaks = lane.service.peaks(lane.job(1)).unwrap();
    let waiting = lane
        .service
        .extract(lane.save(1, 1.0, 0.5))
        .unwrap()
        .request;
    lane.service.load_peaks(lane.job(1)).unwrap();
    assert_ne!(peaks, waiting);
    // Another project: the peaks are given up, the files are not, and what
    // the one under way said is still there to be read.
    lane.service.cancel_all();
    match lane.service.poll() {
        Some(SoundEvent::Progress { request, done }) => assert_eq!((request, done), (saved, 0.5)),
        other => panic!("{other:?}"),
    }
    lane.open();
    assert_eq!(lane.answered(), vec![saved, waiting]);
    assert_eq!(lane.started.load(Ordering::Acquire), 2);

    // Given up by name, under way and waiting, a file answers nothing.
    *lane.gate.0.lock().unwrap() = false;
    let under_way = lane
        .service
        .extract(lane.save(1, 0.0, 1.0))
        .unwrap()
        .request;
    lane.started(3);
    let waiting = lane
        .service
        .extract(lane.save(1, 1.0, 0.5))
        .unwrap()
        .request;
    assert_eq!(lane.service.cancel(waiting), Cancel::GivenUp);
    assert_eq!(lane.service.cancel(under_way), Cancel::GivenUp);
    lane.open();
    assert!(lane.events().is_empty());
    assert_eq!(lane.claimed.load(Ordering::Acquire), 2);

    // A newer file of the same span stops the one under way and names it,
    // once: one given up already is not named again.
    *lane.gate.0.lock().unwrap() = false;
    let under_way = lane
        .service
        .extract(lane.save(1, 0.0, 1.0))
        .unwrap()
        .request;
    lane.started(4);
    let newer = lane.service.extract(lane.save(2, 0.0, 1.0)).unwrap();
    assert_eq!(newer.replaced, vec![under_way]);
    let newest = lane.service.extract(lane.save(3, 0.0, 1.0)).unwrap();
    assert_eq!(newest.replaced, vec![newer.request]);
    lane.open();
    assert_eq!(lane.answered(), vec![newest.request]);
    assert_eq!(lane.claimed.load(Ordering::Acquire), 3);
}

#[test]
fn a_file_that_has_claimed_its_last_step_is_not_given_up() {
    let lane = Gated::new();
    lane.hold_placing(true);
    let saved = lane
        .service
        .extract(lane.save(1, 0.0, 1.0))
        .unwrap()
        .request;
    lane.open();
    lane.wait("the last step", || {
        lane.claimed.load(Ordering::Acquire) == 1
    });
    // Too late by name, by a newer file of the same span and by another
    // project alike: it is being finished, and answers. The newer one does
    // not name it as given up.
    assert_eq!(lane.service.cancel(saved), Cancel::Finishing);
    let newer = lane.service.extract(lane.save(2, 0.0, 1.0)).unwrap();
    assert_eq!(newer.replaced, Vec::<u64>::new());
    let newer = newer.request;
    lane.service.cancel_all();
    lane.hold_placing(false);
    assert_eq!(lane.answered(), vec![saved, newer]);

    // An answer nobody has read yet is not taken back either.
    let saved = lane
        .service
        .extract(lane.save(3, 0.0, 1.0))
        .unwrap()
        .request;
    lane.wait("the answer", || {
        !lane.service.busy() && lane.claimed.load(Ordering::Acquire) == 3
    });
    assert_eq!(lane.service.cancel(saved), Cancel::Written);
    assert_eq!(lane.answered(), vec![saved]);

    // One whose last step fails is not written: until it answers it is
    // being finished, and its failure, unread, is given up with it.
    lane.hold_placing(true);
    let mut failing = lane.save(4, 0.0, 1.0);
    failing.to = lane.dir.path().join("fails.wav");
    let fails = lane.service.extract(failing).unwrap().request;
    lane.wait("the last step", || {
        lane.claimed.load(Ordering::Acquire) == 4
    });
    assert_eq!(lane.service.cancel(fails), Cancel::Finishing);
    lane.hold_placing(false);
    lane.wait("the failure", || !lane.service.busy());
    assert_eq!(lane.service.cancel(fails), Cancel::GivenUp);
    assert!(lane.events().is_empty());
}

#[test]
fn a_part_is_put_in_place_whole_and_otherwise_removed() {
    // Through the handle it is flushed by, which on Windows must be one that
    // may write: a read-only one fails there.
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("take.wav");
    fs::write(&to, b"an earlier take").unwrap();
    let part = extract::Part::make(part_path(&to)).unwrap();
    fs::write(part_path(&to), b"the new take").unwrap();
    part.place(&to).unwrap();
    assert_eq!(fs::read(&to).unwrap(), b"the new take");
    assert!(!part_path(&to).exists());
    // One in the way is refused and left; one not put in place is removed,
    // and the target is left as it was.
    let part = extract::Part::make(part_path(&to)).unwrap();
    assert!(extract::Part::make(part_path(&to)).is_err());
    assert!(part_path(&to).exists());
    drop(part);
    assert!(!part_path(&to).exists());
    assert_eq!(fs::read(&to).unwrap(), b"the new take");
}

#[test]
fn a_file_that_cannot_be_written_is_refused_before_any_process() {
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    let sound = sound_at(&dir.path().join("take.wav"), 64);
    let job = |asset: &Asset, from: f64, span: f64, to: &str| ExtractJob {
        project: Uuid::from_u128(1),
        asset: asset.clone(),
        from: Time::from_seconds(from),
        span: Time::from_seconds(span),
        to: dir.path().join(to),
        keep: Vec::new(),
    };
    let refused = |job: ExtractJob, why: &str| match service.extract(job) {
        Err(MediaError::Invalid(said)) => assert_eq!(said, why),
        other => panic!("{other:?}"),
    };
    let mut silent = Asset::new(
        dir.path().join("silent.mp4"),
        MediaKind::Video,
        Time::from_seconds(2.0),
    );
    (silent.width, silent.height) = (320, 240);
    refused(
        job(&silent, 0.0, 1.0, "a.wav"),
        "Media has no sound to save",
    );
    let none = Asset::new(dir.path().join("none.wav"), MediaKind::Audio, Time::ZERO);
    refused(job(&none, 0.0, 1.0, "a.wav"), "Media has no length");
    refused(job(&sound, 0.0, 0.0, "a.wav"), "The span to save is empty");
    refused(job(&sound, -1.0, 1.0, "a.wav"), "The span to save is empty");
    refused(
        job(&sound, 2.0, 1.0, "a.wav"),
        "The span to save starts after the media ends",
    );
    let mut nameless = job(&sound, 0.0, 1.0, "a.wav");
    nameless.to = PathBuf::from("/");
    refused(nameless, "Choose a file to save the sound to");
    refused(
        job(&sound, 0.0, 1.0, "take.wav"),
        "The sound is not saved over the media it is read from",
    );
    let mut over = job(&sound, 0.0, 1.0, "other.mp4");
    over.keep = vec![dir.path().join("talk.mp4"), dir.path().join("other.mp4")];
    refused(
        over,
        "other.mp4 is media of the project: its sound is not saved over it",
    );
    assert_eq!(service.processes.started_tools(), 0);
    assert!(!service.busy());
    assert!(service.poll().is_none());
    assert_eq!(fs::read(&sound.path).unwrap(), vec![7u8; 64]);
}

/// What the lane says of the file it was asked to write as `request`, and
/// the progress it said on the way, which never goes back or past the end.
fn extracted(service: &SoundService, request: u64) -> SoundEvent {
    let limit = Instant::now() + Duration::from_secs(60);
    let mut said = 0.0;
    loop {
        match service.poll() {
            Some(SoundEvent::Progress { request: of, done }) if of == request => {
                assert!(done >= said && done <= 1.0, "{said} then {done}");
                said = done;
            }
            Some(answer @ SoundEvent::Extracted { request: of, .. }) if of == request => {
                return answer;
            }
            Some(other) => panic!("{other:?}"),
            None => {
                assert!(Instant::now() < limit, "timed out waiting for the file");
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// What the lane says of a file it was asked to write, which leaves no part
/// behind whatever the answer.
fn saved(service: &SoundService, job: ExtractJob) -> Result<Time, String> {
    let (asset, to) = (job.asset.clone(), job.to.clone());
    let request = service.extract(job).unwrap().request;
    let SoundEvent::Extracted {
        project,
        asset_id,
        source,
        to: written,
        result,
        ..
    } = extracted(service, request)
    else {
        unreachable!()
    };
    assert_eq!((project, asset_id), (Uuid::from_u128(1), asset.id));
    assert_eq!(source, SourceIdentity::of(&asset));
    assert_eq!(written, to);
    assert!(!part_path(&to).exists(), "{}", part_path(&to).display());
    result
}

/// The channels, rate and samples of a 16-bit PCM WAV file, read by its
/// chunks: FFmpeg writes a `LIST` chunk before the samples.
fn wav_of(path: &Path) -> (u16, u32, Vec<i16>) {
    let bytes = fs::read(path).unwrap();
    assert_eq!((&bytes[..4], &bytes[8..12]), (&b"RIFF"[..], &b"WAVE"[..]));
    let (mut at, mut format) = (12, None);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " => {
                // PCM, 16 bits a sample.
                assert_eq!(u16::from_le_bytes([body[0], body[1]]), 1);
                assert_eq!(u16::from_le_bytes([body[14], body[15]]), 16);
                format = Some((
                    u16::from_le_bytes([body[2], body[3]]),
                    u32::from_le_bytes(body[4..8].try_into().unwrap()),
                ));
            }
            b"data" => {
                let (channels, rate) = format.expect("the format comes before the samples");
                let samples = (body.chunks_exact(2))
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                return (channels, rate, samples);
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    panic!("{} has no samples", path.display());
}

/// The frequency of a steady tone by the periods between its first and its
/// last rising zero crossing, and its level in dBFS by its root mean square,
/// over samples of one channel.
fn tone_of(samples: &[i16], rate: u32) -> (f64, f64) {
    let rising: Vec<usize> = (samples.windows(2).enumerate())
        .filter(|(_, pair)| pair[0] < 0 && pair[1] >= 0)
        .map(|(at, _)| at)
        .collect();
    let (Some(first), Some(last)) = (rising.first(), rising.last()) else {
        panic!("no tone in {} samples", samples.len());
    };
    let power = samples
        .iter()
        .map(|sample| (f64::from(*sample) / 32768.0).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    (
        (rising.len() - 1) as f64 * f64::from(rate) / (last - first) as f64,
        10.0 * power.log10(),
    )
}

/// One frame at 30 fps, in microseconds: how close a length must be.
const FRAME: i64 = 33_334;

#[test]
fn a_videos_tone_is_written_as_a_wav_of_its_length_and_its_tone() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; no sound was written");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Three seconds of picture with a 1 kHz tone at half of full scale.
    let path = dir.path().join("talk.mp4");
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x48:r=30:d=3",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.5*sin(2*PI*1000*t):s=48000:d=3",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ],
    );
    let asset = probed(&path);
    assert!(asset.kind == MediaKind::Video && asset.has_audio);
    let service = SoundService::new(|| {}).unwrap();
    let to = dir.path().join("talk.wav");
    let length = saved(
        &service,
        ExtractJob::whole(Uuid::from_u128(1), asset.clone(), to.clone()),
    )
    .unwrap();
    // FFmpeg's own reader takes it for sound, as long as the video.
    let written = probed(&to);
    eprintln!(
        "MEASURE: {} s of video wrote {} s of sound, {} said",
        asset.duration.as_seconds(),
        written.duration.as_seconds(),
        length.as_seconds()
    );
    assert_eq!(written.kind, MediaKind::Audio);
    assert!((written.duration.0 - asset.duration.0).abs() <= FRAME);
    assert!((length.0 - asset.duration.0).abs() <= FRAME);
    // The same tone at the same level, away from the coder's two ends.
    let (channels, rate, samples) = wav_of(&to);
    assert_eq!((channels, rate), (1, 48_000));
    let (hertz, level) = tone_of(&samples[24_000..120_000], rate);
    eprintln!("MEASURE: written tone {hertz:.1} Hz at {level:.2} dBFS");
    assert!((hertz - 1000.0).abs() <= 2.0, "{hertz}");
    // Half of full scale is -9.03 dBFS of root mean square.
    assert!((level + 9.03).abs() <= 0.5, "{level}");
    await_reaped(&service);
    assert_eq!(service.processes.started_tools(), 1);
}

#[test]
fn a_clips_span_is_written_and_no_more() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; no span was written");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Two seconds of silence, then two of a tone, in two channels.
    let path = dir.path().join("after.wav");
    let wave = r"0.5*sin(2*PI*1000*t)*gte(t\,2)";
    made(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("aevalsrc={wave}|{wave}:s=48000:d=4"),
            "-c:a",
            "pcm_s16le",
        ],
    );
    let asset = probed(&path);
    let service = SoundService::new(|| {}).unwrap();
    let span = |from: f64, span: f64, name: &str| ExtractJob {
        project: Uuid::from_u128(1),
        asset: asset.clone(),
        from: Time::from_seconds(from),
        span: Time::from_seconds(span),
        to: dir.path().join(name),
        keep: Vec::new(),
    };
    // A second from 1.5 s: half a second of silence, then the tone.
    let length = saved(&service, span(1.5, 1.0, "span.wav")).unwrap();
    assert!((length.0 - 1_000_000).abs() <= FRAME, "{length:?}");
    let written = probed(&dir.path().join("span.wav"));
    assert!((written.duration.0 - 1_000_000).abs() <= FRAME);
    let (channels, rate, samples) = wav_of(&dir.path().join("span.wav"));
    assert_eq!((channels, rate), (2, 48_000));
    let frames = samples.len() / 2;
    assert!((frames as i64 - 48_000).abs() <= 1_600, "{frames} frames");
    // Within 10 ms of where the tone begins, its edge.
    let first = (samples.chunks_exact(2))
        .position(|frame| frame[0].unsigned_abs() > 1_000)
        .expect("the tone is in the span");
    eprintln!("MEASURE: the tone begins {first} frames into the span");
    assert!((first as i64 - 24_000).abs() <= 480, "{first}");
    let (hertz, level) = tone_of(
        &(samples[60_000..].chunks_exact(2))
            .map(|frame| frame[0])
            .collect::<Vec<_>>(),
        rate,
    );
    assert!((hertz - 1000.0).abs() <= 2.0, "{hertz}");
    assert!((level + 9.03).abs() <= 0.2, "{level}");

    // A span that runs past the end is what of it the file has.
    let length = saved(&service, span(3.0, 10.0, "rest.wav")).unwrap();
    assert!((length.0 - 1_000_000).abs() <= FRAME, "{length:?}");
    assert!((probed(&dir.path().join("rest.wav")).duration.0 - 1_000_000).abs() <= FRAME);
    await_reaped(&service);
}

/// Sound that comes only as fast as it is fed: a WAV header and then
/// silence, a tenth of a second of 8 kHz each 10 ms, down a named pipe until
/// whoever reads it stops. Media of a minute, as an import would say, that
/// never ends; the feeder ends with its reader.
#[cfg(unix)]
fn trickle(dir: &Path, name: &str) -> (Asset, thread::JoinHandle<()>) {
    use std::io::Write;
    let path = dir.join(name);
    let made = std::process::Command::new("mkfifo").arg(&path).status();
    assert!(made.is_ok_and(|status| status.success()), "no named pipe");
    let fed = path.clone();
    let feeder = thread::spawn(move || {
        // Opened when FFmpeg opens it, and let go when FFmpeg is stopped.
        let Ok(mut pipe) = fs::OpenOptions::new().write(true).open(&fed) else {
            return;
        };
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&u32::MAX.to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&8_000u32.to_le_bytes());
        header.extend_from_slice(&16_000u32.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&(u32::MAX - 36).to_le_bytes());
        let tenth = vec![0u8; 1_600];
        let mut fed = pipe.write_all(&header);
        while fed.is_ok() {
            fed = pipe.write_all(&tenth);
            thread::sleep(Duration::from_millis(10));
        }
    });
    let mut asset = Asset::new(path, MediaKind::Audio, Time::from_seconds(60.0));
    asset.file_size = 1;
    (asset, feeder)
}

#[cfg(unix)]
#[test]
fn a_file_given_up_or_replaced_leaves_neither_itself_nor_its_part() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; cancelling a written file was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    let job = |asset: &Asset, project: u128, name: &str| {
        ExtractJob::whole(
            Uuid::from_u128(project),
            asset.clone(),
            dir.path().join(name),
        )
    };
    let gone = |name: &str| {
        let path = dir.path().join(name);
        !path.exists() && !part_path(&path).exists()
    };
    // Given up by name at its first word: no file, no part.
    let (asset, feeder) = trickle(dir.path(), "first.pipe");
    let asked = service.extract(job(&asset, 1, "new.wav")).unwrap().request;
    await_progress(&service, asked);
    assert!(part_path(&dir.path().join("new.wav")).exists());
    assert_eq!(service.cancel(asked), Cancel::GivenUp);
    await_reaped(&service);
    feeder.join().unwrap();
    assert!(service.poll().is_none());
    assert!(gone("new.wav"));

    // Over a file that is there, which is left as it was.
    let kept = dir.path().join("kept.wav");
    fs::write(&kept, b"an earlier take").unwrap();
    let (asset, feeder) = trickle(dir.path(), "second.pipe");
    let asked = service.extract(job(&asset, 1, "kept.wav")).unwrap().request;
    await_progress(&service, asked);
    assert_eq!(service.cancel(asked), Cancel::GivenUp);
    await_reaped(&service);
    feeder.join().unwrap();
    assert_eq!(fs::read(&kept).unwrap(), b"an earlier take");
    assert!(!part_path(&kept).exists());

    // Replaced by a newer file of the same span of the same media (its file
    // fed anew): the first is stopped and its part removed. The second goes
    // on through another project's arrival until it is given up by name.
    let (asset, first_feeder) = trickle(dir.path(), "third.pipe");
    let first = service
        .extract(job(&asset, 2, "first.wav"))
        .unwrap()
        .request;
    await_progress(&service, first);
    let (mut again, second_feeder) = trickle(dir.path(), "fourth.pipe");
    again.id = asset.id;
    let second = service
        .extract(job(&again, 3, "second.wav"))
        .unwrap()
        .request;
    await_progress(&service, second);
    first_feeder.join().unwrap();
    assert!(gone("first.wav"));
    service.cancel_all();
    assert!(service.busy());
    await_progress(&service, second);
    assert_eq!(service.cancel(second), Cancel::GivenUp);
    await_reaped(&service);
    second_feeder.join().unwrap();
    assert!(service.poll().is_none());
    assert!(gone("second.wav"));
    assert_eq!(service.processes.started_tools(), 4);
    // Nothing but the pipes and the file that was kept is left.
    let mut left: Vec<_> = (fs::read_dir(dir.path()).unwrap())
        .map(|entry| entry.unwrap().file_name())
        .collect();
    left.sort();
    let pipes = [
        "first.pipe",
        "fourth.pipe",
        "kept.wav",
        "second.pipe",
        "third.pipe",
    ];
    assert_eq!(left, pipes);
}

#[test]
fn a_file_that_is_there_is_replaced_only_by_a_whole_one() {
    let _heavy = crate::heavy_test();
    if !tools_available() {
        eprintln!("SKIP: FFmpeg is unavailable; replacing a file was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let service = SoundService::new(|| {}).unwrap();
    let target = dir.path().join("take.wav");
    fs::write(&target, b"an earlier take").unwrap();
    let job = |asset: &Asset| ExtractJob::whole(Uuid::from_u128(1), asset.clone(), target.clone());

    // An FFmpeg that fails, on a file that is no sound and on one that is
    // not there, leaves it as it was.
    let noise = sound_at(&dir.path().join("noise.wav"), 4096);
    let said = saved(&service, job(&noise)).unwrap_err();
    assert!(
        said.starts_with("ffmpeg: could not write the sound"),
        "{said}"
    );
    let gone = Asset::new(
        dir.path().join("gone.wav"),
        MediaKind::Audio,
        Time::from_seconds(1.0),
    );
    let said = saved(&service, job(&gone)).unwrap_err();
    assert!(said.starts_with("ffmpeg: "), "{said}");
    assert_eq!(fs::read(&target).unwrap(), b"an earlier take");

    // A part file that is in the way is someone's: it is left, and so is
    // the target, and no process is started.
    let part = part_path(&target);
    fs::write(&part, b"someone's").unwrap();
    let asset = tone(dir.path(), 440, 1);
    let request = service.extract(job(&asset)).unwrap().request;
    let SoundEvent::Extracted {
        result: Err(said), ..
    } = extracted(&service, request)
    else {
        panic!("a part in the way was written over");
    };
    assert!(said.contains("is in the way"), "{said}");
    assert_eq!(fs::read(&part).unwrap(), b"someone's");
    fs::remove_file(&part).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"an earlier take");
    assert_eq!(service.processes.started_tools(), 2);

    // The media itself by another name, and a folder, are refused in words.
    fs::create_dir(dir.path().join("sub")).unwrap();
    let mut over = job(&asset);
    over.to = dir.path().join("sub").join("..").join("tone440.wav");
    let said = saved(&service, over).unwrap_err();
    assert_eq!(
        said,
        "The sound is not saved over the media it is read from"
    );
    // Another file of the project's media, by `..`, by a link to its
    // folder, and by letters of another case where the folder ignores case.
    let other = dir.path().join("other.wav");
    fs::write(&other, b"other media").unwrap();
    let mut aliases = vec![dir.path().join("sub").join("..").join("other.wav")];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path(), dir.path().join("link")).unwrap();
        aliases.push(dir.path().join("link").join("other.wav"));
    }
    if dir.path().join("OTHER.WAV").exists() {
        aliases.push(dir.path().join("OTHER.WAV"));
        let mut over = job(&asset);
        over.to = dir.path().join("TONE440.WAV");
        let said = saved(&service, over).unwrap_err();
        assert_eq!(
            said,
            "The sound is not saved over the media it is read from"
        );
    }
    for alias in aliases {
        let mut over = job(&asset);
        over.keep = vec![asset.path.clone(), other.clone()];
        over.to = alias;
        let said = saved(&service, over).unwrap_err();
        assert_eq!(
            said,
            "other.wav is media of the project: its sound is not saved over it"
        );
    }
    assert_eq!(fs::read(&other).unwrap(), b"other media");
    let mut folder = job(&asset);
    folder.to = dir.path().join("folder");
    fs::create_dir(&folder.to).unwrap();
    let said = saved(&service, folder).unwrap_err();
    assert!(said.ends_with("is a folder"), "{said}");
    assert_eq!(service.processes.started_tools(), 2);

    // Written whole, it is replaced.
    let length = saved(&service, job(&asset)).unwrap();
    assert!((length.0 - 1_000_000).abs() <= FRAME);
    let (_, rate, samples) = wav_of(&target);
    assert_eq!(rate, 48_000);
    assert!((samples.len() as i64 - 48_000).abs() <= 1_600);
    await_reaped(&service);
    assert_eq!(service.processes.started_tools(), 3);
}
