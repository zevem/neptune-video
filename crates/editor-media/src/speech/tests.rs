use super::*;
use editor_core::{Asset, Clip, MediaKind, Track, TrackKind};
use std::sync::{Arc, Mutex, atomic::AtomicUsize, mpsc};

fn control() -> Control {
    let lane = Lane {
        running: Some(1),
        ..Lane::default()
    };
    Control {
        shared: Arc::new(Shared {
            lane: Mutex::new(lane),
            ready: Condvar::new(),
            cancelled: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            wake: Arc::new(|| {}),
        }),
        request: 1,
    }
}

fn event(service: &SpeechService) -> SpeechEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = service.poll() {
            return event;
        }
        assert!(Instant::now() < deadline, "no speech event");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_catalog_names_each_file_once_with_its_size_and_sum() {
    let mut files = std::collections::BTreeSet::new();
    let mut last = 0;
    for model in MODELS.iter().chain([&DETECTOR]) {
        assert!(files.insert(model.file), "{} twice", model.file);
        assert!(model.url.starts_with("https://huggingface.co/"));
        assert!(model.url.ends_with(model.file));
        assert_eq!(model.sha256.len(), 64);
        assert!(
            model
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert!(!model.name.is_empty() && !model.note.is_empty());
    }
    for model in &MODELS {
        assert!(model.bytes > last, "{} is not larger", model.id);
        last = model.bytes;
        assert_eq!(Model::by_id(model.id), Some(model));
    }
    assert!(Model::by_id(DEFAULT_MODEL).is_some());
    assert!(Model::by_id("huge").is_none());
    assert_eq!(Model::by_id("base").unwrap().megabytes(), 60);
}

#[test]
fn a_model_counts_as_installed_only_at_its_full_size() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!DETECTOR.installed(dir.path()));
    fs::write(DETECTOR.path(dir.path()), b"short").unwrap();
    assert!(!DETECTOR.installed(dir.path()));
    fs::write(DETECTOR.path(dir.path()), vec![0; DETECTOR.bytes as usize]).unwrap();
    assert!(DETECTOR.installed(dir.path()));
}

#[test]
fn languages_are_whisper_codes_or_nothing() {
    assert_eq!(whisper_language("en"), Some("en"));
    assert_eq!(whisper_language("pt-BR"), Some("pt"));
    assert_eq!(whisper_language("zh_Hant"), Some("zh"));
    assert_eq!(whisper_language("ES"), Some("es"));
    assert_eq!(whisper_language("nb"), Some("no"));
    assert_eq!(whisper_language("nn-NO"), Some("no"));
    assert_eq!(whisper_language("fil"), Some("tl"));
    assert_eq!(whisper_language("iw"), Some("he"));
    assert_eq!(whisper_language("jv"), Some("jw"));
    assert_eq!(whisper_language("yue"), Some("yue"));
    // Names, unknown codes and nothing are not languages to Whisper here.
    assert_eq!(whisper_language("english"), None);
    assert_eq!(whisper_language("zz"), None);
    assert_eq!(whisper_language("x"), None);
    assert_eq!(whisper_language(""), None);
    assert_eq!(whisper_language("e1"), None);
}

fn header(fields: [i32; 11]) -> Vec<u8> {
    let mut bytes = 0x6767_6d6c_u32.to_le_bytes().to_vec();
    for field in fields {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(&[0; 64]);
    bytes
}

fn shape(fields: [i32; 11]) -> Result<ModelShape, MediaError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.bin");
    fs::write(&path, header(fields)).unwrap();
    ModelShape::read(&path)
}

#[test]
fn a_model_header_picks_the_heads_that_time_its_words() {
    // vocabulary, audio ctx, audio state, audio heads, audio layers, text
    // ctx, text state, text heads, text layers, mels, ftype
    let tiny = shape([51_865, 1500, 384, 6, 4, 448, 384, 6, 4, 80, 9]).unwrap();
    assert!(tiny.multilingual());
    assert!(matches!(
        tiny.alignment(),
        DtwMode::ModelPreset {
            model_preset: DtwModelPreset::Tiny
        }
    ));
    let tiny_en = shape([51_864, 1500, 384, 6, 4, 448, 384, 6, 4, 80, 9]).unwrap();
    assert!(!tiny_en.multilingual());
    assert!(matches!(
        tiny_en.alignment(),
        DtwMode::ModelPreset {
            model_preset: DtwModelPreset::TinyEn
        }
    ));
    let turbo = shape([51_866, 1500, 1280, 20, 32, 448, 1280, 20, 4, 128, 8]).unwrap();
    assert!(matches!(
        turbo.alignment(),
        DtwMode::ModelPreset {
            model_preset: DtwModelPreset::LargeV3Turbo
        }
    ));
    let v3 = shape([51_866, 1500, 1280, 20, 32, 448, 1280, 20, 32, 128, 8]).unwrap();
    assert!(matches!(
        v3.alignment(),
        DtwMode::ModelPreset {
            model_preset: DtwModelPreset::LargeV3
        }
    ));
    let v2 = shape([51_865, 1500, 1280, 20, 32, 448, 1280, 20, 32, 80, 8]).unwrap();
    assert!(matches!(
        v2.alignment(),
        DtwMode::ModelPreset {
            model_preset: DtwModelPreset::LargeV2
        }
    ));
    // A fine-tune of a shape whisper.cpp does not name uses its top layers.
    let other = shape([51_865, 1500, 640, 10, 8, 448, 640, 10, 3, 80, 1]).unwrap();
    assert!(matches!(other.alignment(), DtwMode::TopMost { n_top: 2 }));
}

#[test]
fn a_file_that_is_not_a_model_is_refused_before_whisper_opens_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, vec![b'a'; 100]).unwrap();
    assert!(matches!(
        ModelShape::read(&path),
        Err(MediaError::Invalid(_))
    ));
    fs::write(&path, b"lmgg").unwrap();
    assert!(matches!(
        ModelShape::read(&path),
        Err(MediaError::Io { .. })
    ));
    assert!(matches!(
        ModelShape::read(&dir.path().join("missing.bin")),
        Err(MediaError::Io { .. })
    ));
    assert!(matches!(
        shape([51_865, 1500, 384, 6, 0, 448, 384, 6, 4, 80, 9]),
        Err(MediaError::Invalid(_))
    ));
    assert!(matches!(
        shape([51_865, 1500, 384, 6, 4, 448, 384, 6, 900, 80, 9]),
        Err(MediaError::Invalid(_))
    ));
}

// ---- Timing ----------------------------------------------------------------

#[test]
fn condensed_speech_maps_back_to_where_it_was_said() {
    let samples: Vec<f32> = (0..100).map(|i| i as f32).collect();
    let (heard, map) = timing::Condensed::build(&samples, &[10..20, 50..55, 98..140], 3);
    assert_eq!(heard.len(), 10 + 3 + 5 + 3 + 2 + 3);
    assert_eq!(&heard[..10], &samples[10..20]);
    assert_eq!(&heard[10..13], &[0.0; 3]);
    assert_eq!(&heard[13..18], &samples[50..55]);
    assert_eq!(map.original(0), 10);
    assert_eq!(map.original(9), 19);
    // The silence put after a stretch is its end.
    assert_eq!(map.original(11), 20);
    assert_eq!(map.original(13), 50);
    assert_eq!(map.original(17), 54);
    assert_eq!(map.original(21), 98);
    assert_eq!(map.original(1_000), 100);
    let (heard, map) = timing::Condensed::build(&samples, &[], 3);
    assert!(heard.is_empty());
    assert_eq!(map.original(5), 0);
}

#[test]
fn a_long_sound_is_cut_in_its_quietest_moment_before_the_limit() {
    let mut samples = vec![0.5_f32; 1000];
    samples[700..740].fill(0.0);
    // Of the silent windows the latest, 720 to 740, is cut in its middle.
    assert_eq!(timing::quiet_cut(&samples, 900, 400, 20), 730);
    // Too little to search: the limit itself.
    assert_eq!(timing::quiet_cut(&samples, 10, 400, 20), 10);
    assert_eq!(timing::quiet_cut(&samples, 900, 400, 0), 900);
    // Equal quiet everywhere keeps the latest window.
    let flat = vec![0.0_f32; 1000];
    assert_eq!(timing::quiet_cut(&flat, 900, 400, 20), 890);
    // The limit is never past the samples.
    assert!(timing::quiet_cut(&samples, 5_000, 400, 20) <= 1000);
}

fn piece(text: &str, at: i64, segment: usize, segment_end: i64) -> timing::Piece {
    timing::Piece {
        bytes: text.as_bytes().to_vec(),
        at,
        segment,
        segment_end,
    }
}

#[test]
fn whisper_tokens_become_words_with_times_on_the_timeline() {
    let samples = vec![0.0_f32; 1600 * 10];
    // Speech at 1 s to 3 s and 6 s to 8 s of the piece, heard with 1 s apart.
    let (_, map) = timing::Condensed::build(&samples, &[1600..4800, 9600..12800], 1600);
    let pieces = [
        piece(" Hello", 0, 0, 150),
        piece(" world", 100, 0, 150),
        // The second stretch is heard from 3 s.
        piece(" again", 300, 1, 400),
    ];
    let offset = Time::from_seconds(10.0);
    let bounds = offset..Time::from_seconds(30.0);
    let words = timing::spoken(&pieces, &map, offset, 1600, bounds);
    let texts: Vec<_> = words.iter().map(|word| word.text.as_str()).collect();
    assert_eq!(texts, [" Hello", " world", " again"]);
    assert_eq!(words[0].at, Time::from_seconds(11.0));
    // A word lasts no longer than a word of its length is said for...
    assert_eq!(words[0].end, Time::from_seconds(11.55));
    assert_eq!(words[1].at, Time::from_seconds(12.0));
    // ...nor past its segment's end...
    assert_eq!(words[1].end, Time::from_seconds(12.5));
    assert_eq!(words[2].at, Time::from_seconds(16.0));
    assert_eq!(words[2].end, Time::from_seconds(16.55));
    // ...nor past the next word.
    let close = [piece(" Hello", 0, 0, 150), piece(" world", 20, 0, 150)];
    let words = timing::spoken(&close, &map, offset, 1600, offset..Time::from_seconds(30.0));
    assert_eq!(words[0].end, Time::from_seconds(11.2));
}

#[test]
fn a_character_split_between_tokens_is_one_word() {
    let samples = vec![0.0_f32; 16_000];
    let (_, map) = timing::Condensed::build(&samples, std::slice::from_ref(&(0..16_000)), 0);
    let heart = "❤".as_bytes();
    let pieces = [
        timing::Piece {
            bytes: [b" ".as_slice(), &heart[..1]].concat(),
            at: 10,
            segment: 0,
            segment_end: 90,
        },
        timing::Piece {
            bytes: heart[1..].to_vec(),
            at: 20,
            segment: 0,
            segment_end: 90,
        },
        piece(" ok", 40, 0, 90),
        // A character left unfinished by its segment is not lost.
        timing::Piece {
            bytes: heart[..2].to_vec(),
            at: 60,
            segment: 0,
            segment_end: 90,
        },
        piece(" next", 70, 1, 95),
    ];
    let bounds = Time::ZERO..Time::from_seconds(1.0);
    let words = timing::spoken(&pieces, &map, Time::ZERO, 16_000, bounds);
    let texts: Vec<_> = words.iter().map(|word| word.text.as_str()).collect();
    assert_eq!(texts, [" ❤", " ok", "\u{FFFD}", " next"]);
    assert_eq!(words[0].at, Time::from_seconds(0.1));
    assert_eq!(words[0].end, Time::from_seconds(0.33));
}

#[test]
fn word_times_stay_in_order_and_inside_the_stretch() {
    let samples = vec![0.0_f32; 16_000];
    let (_, map) = timing::Condensed::build(&samples, std::slice::from_ref(&(0..16_000)), 0);
    let pieces = [
        piece(" one", 50, 0, 100),
        // DTW can put a token before the one it follows.
        piece(" two", 30, 0, 100),
        piece(" three", 95, 0, 100),
    ];
    let bounds = Time::from_seconds(0.2)..Time::from_seconds(0.9);
    let words = timing::spoken(&pieces, &map, Time::ZERO, 16_000, bounds.clone());
    for pair in words.windows(2) {
        assert!(pair[0].at <= pair[1].at);
    }
    for word in &words {
        assert!(bounds.start <= word.at && word.at <= word.end && word.end <= bounds.end);
    }
    assert_eq!(words[1].at, words[0].at);
    assert_eq!(words[2].at, Time::from_seconds(0.9));
}

#[test]
fn sound_descriptions_are_not_speech() {
    for text in [
        " [Music]",
        "(applause)",
        " *laughs*",
        " ♪ ♪",
        "...",
        "",
        "  ",
    ] {
        assert!(timing::annotation(text), "{text:?}");
    }
    for text in [" Hello.", " [Music] plays", " ♪ la la ♪"] {
        assert!(!timing::annotation(text), "{text:?}");
    }
}

// ---- The worker ------------------------------------------------------------

#[test]
fn one_job_runs_at_a_time_and_each_ends_in_one_event() {
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&wakes);
    let service = SpeechService::with_runner(
        Arc::new(move || {
            counted.fetch_add(1, Ordering::AcqRel);
        }),
        Processes::default(),
        Arc::new(move |job, control| {
            control.report(Stage::Recognizing, 0.25);
            released.lock().unwrap().recv().unwrap();
            match job {
                SpeechJob::Download { model, .. } => Ok(Outcome::Downloaded(model)),
                SpeechJob::Transcribe(_) => Err(MediaError::Invalid("No speech".into())),
            }
        }),
    )
    .unwrap();
    let first = service
        .start(SpeechJob::Download {
            model: &MODELS[0],
            dir: PathBuf::from("unused"),
        })
        .unwrap();
    assert!(service.busy());
    assert!(matches!(
        service.start(SpeechJob::Download {
            model: &MODELS[1],
            dir: PathBuf::from("unused"),
        }),
        Err(MediaError::Busy)
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.progress().is_none() {
        assert!(Instant::now() < deadline, "no progress");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        service.progress(),
        Some(SpeechProgress {
            request: first,
            stage: Stage::Recognizing,
            done: 0.25
        })
    );
    release.send(()).unwrap();
    match event(&service) {
        SpeechEvent::Downloaded { request, model } => {
            assert_eq!(request, first);
            assert_eq!(model.id, MODELS[0].id);
        }
        other => panic!("{other:?}"),
    }
    assert!(!service.busy());
    assert_eq!(service.progress(), None);
    assert!(wakes.load(Ordering::Acquire) >= 2);

    let project = Arc::new(Project::new("Speech"));
    let second = service
        .start(SpeechJob::Transcribe(Transcription {
            project,
            start: Time::ZERO,
            end: Time::from_seconds(1.0),
            model: PathBuf::from("unused"),
            detector: None,
            language: None,
            translate: false,
        }))
        .unwrap();
    assert!(second > first);
    release.send(()).unwrap();
    match event(&service) {
        SpeechEvent::Failed { request, message } => {
            assert_eq!(request, second);
            assert_eq!(message, "No speech");
        }
        other => panic!("{other:?}"),
    }
    assert!(service.poll().is_none());
}

#[test]
fn a_cancelled_job_answers_cancelled_and_never_with_what_it_made() {
    let started = Arc::new(AtomicBool::new(false));
    let running = Arc::clone(&started);
    let service = SpeechService::with_runner(
        Arc::new(|| {}),
        Processes::default(),
        Arc::new(move |job, control| {
            running.store(true, Ordering::Release);
            // A job that misses the cancel still is not delivered.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !control.cancelled() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            match job {
                SpeechJob::Download { model, .. } => Ok(Outcome::Downloaded(model)),
                SpeechJob::Transcribe(_) => Ok(Outcome::Transcribed(Vec::new(), None)),
            }
        }),
    )
    .unwrap();
    let request = service
        .start(SpeechJob::Download {
            model: &MODELS[0],
            dir: PathBuf::from("unused"),
        })
        .unwrap();
    while !started.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(2));
    }
    let asked = Instant::now();
    service.cancel(request);
    assert!(matches!(event(&service), SpeechEvent::Cancelled { request: r } if r == request));
    assert!(asked.elapsed() < Duration::from_secs(2));
    // A cancel for a finished request changes nothing for the next one.
    service.cancel(request);
    let next = service
        .start(SpeechJob::Download {
            model: &MODELS[0],
            dir: PathBuf::from("unused"),
        })
        .unwrap();
    service.cancel(next);
    assert!(matches!(event(&service), SpeechEvent::Cancelled { request: r } if r == next));
}

#[test]
fn dropping_the_service_does_not_wait_for_its_job() {
    let (entered, entering) = mpsc::channel();
    let service = SpeechService::with_runner(
        Arc::new(|| {}),
        Processes::default(),
        Arc::new(move |job, control| {
            let _ = entered.send(());
            while !control.cancelled() {
                thread::sleep(Duration::from_millis(2));
            }
            match job {
                SpeechJob::Download { model, .. } => Ok(Outcome::Downloaded(model)),
                SpeechJob::Transcribe(_) => Err(MediaError::Cancelled),
            }
        }),
    )
    .unwrap();
    service
        .start(SpeechJob::Download {
            model: &MODELS[0],
            dir: PathBuf::from("unused"),
        })
        .unwrap();
    entering.recv_timeout(Duration::from_secs(5)).unwrap();
    let dropped = Instant::now();
    drop(service);
    assert!(dropped.elapsed() < Duration::from_millis(500));
}

#[test]
fn an_empty_or_overlong_stretch_is_refused_before_the_model_is_read() {
    let job = |start: Time, end: Time| Transcription {
        project: Arc::new(Project::new("Speech")),
        start,
        end,
        model: PathBuf::from("missing.bin"),
        detector: None,
        language: None,
        translate: false,
    };
    let control = control();
    let processes = Processes::default();
    let empty = transcribe(job(Time::ZERO, Time::ZERO), &control, &processes);
    assert!(matches!(empty, Err(MediaError::Invalid(message)) if message.contains("nothing")));
    let long = transcribe(job(Time::ZERO, MAX_SPEECH + Time(1)), &control, &processes);
    assert!(matches!(long, Err(MediaError::Invalid(message)) if message.contains("six hours")));
    let missing = transcribe(
        job(Time::ZERO, Time::from_seconds(1.0)),
        &control,
        &processes,
    );
    assert!(matches!(missing, Err(MediaError::Io { .. })));
}

// ---- Downloads -------------------------------------------------------------

#[cfg(unix)]
mod downloads {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A curl that serves `body` from where the part already ends, as
    /// `--continue-at -` asks.
    fn fake_curl(dir: &Path, script: &str) -> PathBuf {
        let path = dir.join("curl");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nfor arg; do\n  if [ \"$take\" = 1 ]; then out=\"$arg\"; take=; fi\n  [ \"$arg\" = --output ] && take=1\ndone\n{script}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn serving(dir: &Path, body: &Path) -> PathBuf {
        fake_curl(
            dir,
            &format!(
                "have=$(wc -c < \"$out\" 2>/dev/null || echo 0)\ntail -c +$((have + 1)) '{}' >> \"$out\"",
                body.display()
            ),
        )
    }

    fn file(body: &[u8]) -> &'static Model {
        let sum = Sha256::digest(body);
        let sum: String = sum.iter().map(|byte| format!("{byte:02x}")).collect();
        Box::leak(Box::new(Model {
            id: "test",
            name: "Test model",
            note: "For tests",
            file: "ggml-test.bin",
            url: "https://example.invalid/ggml-test.bin",
            bytes: body.len() as u64,
            sha256: Box::leak(sum.into_boxed_str()),
        }))
    }

    #[test]
    fn a_download_is_checked_then_kept_under_its_own_name() {
        let tools = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..300_000_u32).map(|i| (i % 251) as u8).collect();
        let source = tools.path().join("body");
        fs::write(&source, &body).unwrap();
        let curl = serving(tools.path(), &source);
        let model = file(&body);
        let control = control();
        fetch(model, models.path(), &control, &curl, "Test", 0.0..0.5).unwrap();
        assert_eq!(fs::read(model.path(models.path())).unwrap(), body);
        assert!(model.installed(models.path()));
        assert!(!models.path().join("ggml-test.bin.part").exists());
        let progress = control.shared.lane.lock().unwrap().progress.unwrap();
        assert_eq!(progress.stage, Stage::Checking);
        assert_eq!(progress.done, 0.5);
    }

    #[test]
    fn a_download_carries_on_from_the_part_an_earlier_one_left() {
        let tools = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..200_000_u32).map(|i| (i % 241) as u8).collect();
        let source = tools.path().join("body");
        fs::write(&source, &body).unwrap();
        fs::write(models.path().join("ggml-test.bin.part"), &body[..123_456]).unwrap();
        let curl = serving(tools.path(), &source);
        let model = file(&body);
        fetch(model, models.path(), &control(), &curl, "Test", 0.0..1.0).unwrap();
        assert_eq!(fs::read(model.path(models.path())).unwrap(), body);
    }

    #[test]
    fn a_damaged_download_is_removed_and_never_kept() {
        let tools = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let body = vec![7_u8; 50_000];
        let mut damaged = body.clone();
        damaged[25_000] = 8;
        let source = tools.path().join("body");
        fs::write(&source, &damaged).unwrap();
        let curl = serving(tools.path(), &source);
        let model = file(&body);
        let error = fetch(model, models.path(), &control(), &curl, "Test", 0.0..1.0).unwrap_err();
        assert!(error.to_string().contains("damaged"), "{error}");
        assert!(!model.path(models.path()).exists());
        assert!(!models.path().join("ggml-test.bin.part").exists());
        // A part longer than the file is thrown away, not trusted.
        fs::write(&source, &body).unwrap();
        fs::write(models.path().join("ggml-test.bin.part"), vec![1; 60_000]).unwrap();
        fetch(model, models.path(), &control(), &curl, "Test", 0.0..1.0).unwrap();
        assert_eq!(fs::read(model.path(models.path())).unwrap(), body);
    }

    #[test]
    fn what_curl_says_when_it_fails_is_what_the_person_reads() {
        let tools = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let curl = fake_curl(
            tools.path(),
            "echo 'curl: (6) Could not resolve host: huggingface.co' >&2\nexit 6",
        );
        let error = fetch(
            file(b"x"),
            models.path(),
            &control(),
            &curl,
            "Test",
            0.0..1.0,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "Cannot download Test model: (6) Could not resolve host: huggingface.co"
        );
        let silent = fake_curl(tools.path(), "exit 22");
        let error = fetch(
            file(b"x"),
            models.path(),
            &control(),
            &silent,
            "Test",
            0.0..1.0,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.starts_with("Cannot download Test model: curl stopped"),
            "{error}"
        );
        let missing = fetch(
            file(b"x"),
            models.path(),
            &control(),
            &tools.path().join("no-curl"),
            "Test",
            0.0..1.0,
        )
        .unwrap_err();
        assert_eq!(missing.to_string(), CURL_MISSING);
    }

    #[test]
    fn a_cancel_stops_curl_at_once() {
        let tools = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let curl = fake_curl(tools.path(), "printf abc > \"$out\"\nexec sleep 30");
        let control = control();
        let shared = Arc::clone(&control.shared);
        let cancel = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            shared.cancelled.store(1, Ordering::Release);
        });
        let started = Instant::now();
        let result = fetch(
            file(&[0; 10]),
            models.path(),
            &control,
            &curl,
            "Test",
            0.0..1.0,
        );
        cancel.join().unwrap();
        assert!(matches!(result, Err(MediaError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(3));
        // The part stays for the next attempt to carry on from.
        assert_eq!(
            fs::read(models.path().join("ggml-test.bin.part")).unwrap(),
            b"abc"
        );
    }

    #[test]
    fn a_download_fetches_only_what_is_missing() {
        let tools = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        // The detector is there; a curl asked for it would fail the test.
        fs::write(
            DETECTOR.path(models.path()),
            vec![0; DETECTOR.bytes as usize],
        )
        .unwrap();
        let body = vec![3_u8; 4096];
        let source = tools.path().join("body");
        fs::write(&source, &body).unwrap();
        let curl = fake_curl(
            tools.path(),
            &format!(
                "case \"$out\" in *silero*) exit 9;; esac\ncat '{}' > \"$out\"",
                source.display()
            ),
        );
        let model = file(&body);
        let outcome = download(model, models.path(), &control(), &curl).unwrap();
        assert!(matches!(outcome, Outcome::Downloaded(done) if done.id == "test"));
        assert!(model.installed(models.path()));
        // Once both are there nothing is fetched at all.
        let refusing = fake_curl(tools.path(), "exit 9");
        download(model, models.path(), &control(), &refusing).unwrap();
    }
}

// ---- Hearing the timeline --------------------------------------------------

fn wav(path: &Path, rate: u32, samples: &[i16]) {
    let data = (samples.len() * 2) as u32;
    let mut wave = Vec::with_capacity(44 + data as usize);
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + data).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16_u32.to_le_bytes());
    wave.extend_from_slice(&1_u16.to_le_bytes());
    wave.extend_from_slice(&1_u16.to_le_bytes());
    wave.extend_from_slice(&rate.to_le_bytes());
    wave.extend_from_slice(&(rate * 2).to_le_bytes());
    wave.extend_from_slice(&2_u16.to_le_bytes());
    wave.extend_from_slice(&16_u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        wave.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, wave).unwrap();
}

fn timeline(asset: &Asset, at: Time) -> Project {
    let mut project = Project::new("Speech fixture");
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let mut track = Track::new("Voice", TrackKind::Audio);
    track.clips.push(Clip::from_asset(asset, at));
    project.tracks = vec![Arc::new(track)];
    project.validate().unwrap();
    project
}

#[test]
fn the_timeline_is_heard_mono_at_sixteen_kilohertz_from_where_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("steady.wav");
    wav(&path, 48_000, &vec![8192; 48_000]);
    let asset = Asset::new(path, MediaKind::Audio, Time::from_seconds(1.0));
    let project = Arc::new(timeline(&asset, Time::from_seconds(0.5)));
    let mut mix = OfflineMix::new(
        project,
        Time::from_seconds(0.25),
        SPEECH_RATE,
        Processes::default(),
    );
    let mut samples = Vec::new();
    while samples.len() < SPEECH_RATE as usize * 3 / 2 {
        samples.extend(mix.next().unwrap());
    }
    // Silence until the clip at 0.5 s, then its level, then silence after
    // it ends at 1.5 s.
    let at = |seconds: f64| samples[(seconds * f64::from(SPEECH_RATE)) as usize];
    assert_eq!(at(0.1), 0.0);
    assert!((at(0.5) - 0.25).abs() < 0.01, "{}", at(0.5));
    assert!((at(1.0) - 0.25).abs() < 0.01, "{}", at(1.0));
    assert_eq!(at(1.3), 0.0);
}

/// Transcribes a recording put two seconds into a timeline with the model
/// at `NEPTUNE_VIDEO_WHISPER_MODEL`, and the speech detector at
/// `NEPTUNE_VIDEO_WHISPER_DETECTOR` when it is set. The recording is
/// `NEPTUNE_VIDEO_SPEECH_SAMPLE`, a WAV of someone speaking.
#[test]
fn a_real_model_writes_timed_words_for_speech_on_the_timeline() {
    let (Some(model), Some(sample)) = (
        std::env::var_os("NEPTUNE_VIDEO_WHISPER_MODEL"),
        std::env::var_os("NEPTUNE_VIDEO_SPEECH_SAMPLE"),
    ) else {
        eprintln!("SKIP: set NEPTUNE_VIDEO_WHISPER_MODEL and NEPTUNE_VIDEO_SPEECH_SAMPLE");
        return;
    };
    let detector = std::env::var_os("NEPTUNE_VIDEO_WHISPER_DETECTOR").map(PathBuf::from);
    let sample = PathBuf::from(sample);
    // A plain WAV's length from its header's byte rate.
    let header = fs::read(&sample).unwrap();
    let byte_rate = u32::from_le_bytes(header[28..32].try_into().unwrap());
    let length = Time::from_seconds((header.len() - 44) as f64 / f64::from(byte_rate));
    let asset = Asset::new(sample, MediaKind::Audio, length);
    let start = Time::from_seconds(2.0);
    let project = Arc::new(timeline(&asset, start));
    let end = project.duration() + Time::from_seconds(1.0);
    let job = Transcription {
        project,
        start: Time::ZERO,
        end,
        model: PathBuf::from(model),
        detector,
        language: std::env::var("NEPTUNE_VIDEO_SPEECH_LANGUAGE").ok(),
        translate: false,
    };
    let control = control();
    let begun = Instant::now();
    let outcome = transcribe(job.clone(), &control, &Processes::default()).unwrap();
    let Outcome::Transcribed(words, language) = outcome else {
        panic!("not a transcription");
    };
    let text: String = words.iter().map(|word| word.text.as_str()).collect();
    eprintln!(
        "{:.2} s for {:.2} s heard, language {language:?}:{text}",
        begun.elapsed().as_secs_f64(),
        end.as_seconds()
    );
    for word in &words {
        eprintln!(
            "  {:6.2} {:6.2} {}",
            word.at.as_seconds(),
            word.end.as_seconds(),
            word.text
        );
    }
    assert!(words.len() >= 3, "{text}");
    for pair in words.windows(2) {
        assert!(pair[0].at <= pair[1].at);
    }
    for word in &words {
        assert!(word.at >= start - Time::from_seconds(0.3), "{word:?}");
        assert!(word.at <= word.end && word.end <= end, "{word:?}");
    }
    if let Ok(expected) = std::env::var("NEPTUNE_VIDEO_SPEECH_EXPECT") {
        let heard = text.to_lowercase();
        for word in expected.split(',') {
            assert!(heard.contains(word), "{word} not in{text}");
        }
    }

    // A cancel while whisper.cpp works stops it at its next check, at the
    // end of a pass of its encoder or decoder. Telling the language is a
    // pass it cannot stop in.
    let control = super::tests::control();
    let shared = Arc::clone(&control.shared);
    let cancel = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while shared.lane.lock().unwrap().progress.map(|p| p.stage) != Some(Stage::Recognizing) {
            assert!(Instant::now() < deadline, "never recognizing");
            thread::sleep(Duration::from_millis(1));
        }
        thread::sleep(Duration::from_millis(100));
        shared.cancelled.store(1, Ordering::Release);
        Instant::now()
    });
    let result = transcribe(job, &control, &Processes::default());
    let asked = cancel.join().unwrap();
    assert!(
        matches!(result, Err(MediaError::Cancelled)),
        "{:?}",
        result.err()
    );
    eprintln!(
        "stopped {:.2} s after the cancel",
        asked.elapsed().as_secs_f64()
    );
    assert!(asked.elapsed() < Duration::from_secs(30));
}
