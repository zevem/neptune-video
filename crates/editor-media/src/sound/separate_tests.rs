//! The separation job with a stand-in separator: a shell script that answers
//! `--help` as Demucs does and writes two short WAVs where Demucs writes its
//! stems. No Python and no model; the stems are probed by the real ffprobe.
use super::*;
use crate::{SoundEvent, SoundService, process::tool_command};

#[test]
fn the_tools_progress_and_last_words_are_read_from_its_error_pipe() {
    // tqdm rewrites its bar with carriage returns; the last one counts.
    let bar = "Separating\r  0%|          | 0/5\r 37%|███▋      | 2/5 [00:01<00:02]\r100%|██████████| 5/5";
    assert_eq!(said_progress(bar), Some(1.0));
    assert_eq!(
        said_progress(&bar[..bar.rfind("\r100").unwrap()]),
        Some(0.37)
    );
    assert_eq!(said_progress("Separated tracks will be stored in /x"), None);
    assert_eq!(said_progress("%|"), None);
    assert_eq!(said_progress("250%|"), None);

    let traceback =
        "Traceback (most recent call last):\n  File \"x\"\nRuntimeError: out of memory\n\n";
    assert_eq!(
        last_line(traceback).as_deref(),
        Some("RuntimeError: out of memory")
    );
    assert_eq!(last_line(" 50%|#####\r").as_deref(), Some("50%|#####"));
    assert_eq!(last_line("\n \r"), None);
    let long = "é".repeat(300);
    let cut = last_line(&long).unwrap();
    assert!(cut.ends_with('…') && cut.len() <= 403, "{}", cut.len());
}

#[test]
fn a_missing_separator_is_found_at_probe_time_and_said_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let error = Separator::probe(dir.path().join("demucs")).unwrap_err();
    let said = error.to_string();
    assert!(said.starts_with("No stem separator was found"), "{said}");
    assert!(separator_absent(&said));
    assert!(said.contains(SEPARATOR_VARIABLE), "{said}");
    // FFmpeg's words are not used for it.
    assert!(!said.contains("ffprobe"), "{said}");
}

#[test]
fn media_without_sound_or_a_folder_is_refused_before_any_process() {
    let separator = Separator {
        program: "demucs".into(),
    };
    let job = |asset: Asset, to: &str| SeparateJob {
        project: Uuid::nil(),
        asset,
        to: to.into(),
        separator: separator.clone(),
    };
    let silent = Asset::new("/m/silent.mp4".into(), MediaKind::Video, Time(1_000_000));
    assert!(check(&job(silent, "/m/media")).is_err());
    let mut empty = Asset::new("/m/song.wav".into(), MediaKind::Audio, Time::ZERO);
    empty.has_audio = true;
    assert!(check(&job(empty.clone(), "/m/media")).is_err());
    empty.duration = Time(1_000_000);
    assert!(check(&job(empty.clone(), "/")).is_err());
    assert!(check(&job(empty, "/m/media")).is_ok());
}

#[test]
fn stems_are_named_for_the_media_and_numbered_past_names_that_are_taken() {
    let mut asset = Asset::new("/m/My song.mp3".into(), MediaKind::Audio, Time(1));
    asset.name = "My song.mp3".into();
    let job = SeparateJob {
        project: Uuid::nil(),
        asset,
        to: "/m/Film media".into(),
        separator: Separator {
            program: "demucs".into(),
        },
    };
    assert_eq!(
        job.stem_path("Vocals", 1),
        Path::new("/m/Film media/My song (Vocals).wav")
    );
    assert_eq!(
        job.stem_path("Instruments", 3),
        Path::new("/m/Film media/My song (Instruments) 3.wav")
    );
    assert_eq!(
        job.part_folder(),
        Path::new("/m/Film media/My song (stems).part")
    );
}

// ---- With a stand-in separator -----------------------------------------------------

#[cfg(unix)]
mod fake {
    use super::*;
    use crate::sound::Cancel;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;

    /// What the stand-in does once it has read its arguments as Demucs
    /// takes them: `$out` is the folder after `-o`, `$src` the media.
    const WRITES: &str = r#"mkdir -p "$out/htdemucs"
printf '\r 50%%|#####     | 1/2 [00:01<00:01]' >&2
# More than a pipe holds, before any stem: nothing may wait for it to be read.
i=0; while [ $i -lt 2000 ]; do echo "Separating track $src, a line of output to fill the pipe"; i=$((i+1)); done
sleep 1
cp "$src" "$out/htdemucs/vocals.wav"
cp "$src" "$out/htdemucs/no_vocals.wav"
echo "Separated tracks will be stored in $out""#;

    /// Writes a stem, says it has begun and runs on until it is stopped.
    const RUNS_ON: &str = r#"mkdir -p "$out/htdemucs"
cp "$src" "$out/htdemucs/vocals.wav"
touch "$out/../begun"
exec sleep 30"#;

    const FAILS: &str = r#"mkdir -p "$out/htdemucs"
cp "$src" "$out/htdemucs/vocals.wav"
echo "Traceback (most recent call last):" >&2
echo "RuntimeError: out of memory" >&2
exit 3"#;

    /// An executable shell script in `dir`.
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A separator script in `dir` that does `body`.
    fn separator(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let script = format!(
            r#"#!/bin/sh
case "$1" in --help) echo "usage: demucs [-n NAME] [-o OUT] [--two-stems STEM] [--filename FILENAME] tracks"; exit 0;; esac
out=""; src=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2;;
    -n|--two-stems|--filename) shift 2;;
    *) src="$1"; shift;;
  esac
done
{body}
"#
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn tools_available() -> bool {
        tool_command("ffprobe")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Half a second of a 440 Hz tone, mono 16-bit at 8 kHz, as a WAV.
    fn song(dir: &Path) -> Asset {
        let (rate, frames) = (8_000u32, 4_000u32);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(frames * 2).to_le_bytes());
        for frame in 0..frames {
            let phase = f64::from(frame) / f64::from(rate) * 440.0 * std::f64::consts::TAU;
            bytes.extend_from_slice(&((phase.sin() * 8_000.0) as i16).to_le_bytes());
        }
        let path = dir.join("song.wav");
        fs::write(&path, bytes).unwrap();
        let mut asset = Asset::new(path, MediaKind::Audio, Time(500_000));
        asset.has_audio = true;
        asset
    }

    struct Bench {
        dir: tempfile::TempDir,
        asset: Asset,
        to: PathBuf,
        service: SoundService,
    }

    fn bench() -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let asset = song(dir.path());
        let to = dir.path().join("Film media");
        let service = SoundService::new(|| {}).unwrap();
        Bench {
            dir,
            asset,
            to,
            service,
        }
    }

    impl Bench {
        fn job(&self, program: &Path) -> SeparateJob {
            SeparateJob {
                project: Uuid::from_u128(7),
                asset: self.asset.clone(),
                to: self.to.clone(),
                separator: Separator::probe(program).unwrap(),
            }
        }

        /// What the lane said until it was idle, read as it was said: a
        /// job's progress goes once it has answered.
        fn events(&self) -> Vec<SoundEvent> {
            let until = Instant::now() + Duration::from_secs(20);
            let mut events = Vec::new();
            while self.service.busy() {
                assert!(Instant::now() < until, "the lane did not finish");
                events.extend(std::iter::from_fn(|| self.service.poll()));
                thread::sleep(Duration::from_millis(5));
            }
            events.extend(std::iter::from_fn(|| self.service.poll()));
            events
        }

        /// The files in the media folder, by name.
        fn kept(&self) -> Vec<String> {
            let Ok(entries) = fs::read_dir(&self.to) else {
                return Vec::new();
            };
            let mut names: Vec<String> = (entries.flatten())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    fn separated(events: Vec<SoundEvent>, request: u64) -> Result<Vec<Asset>, String> {
        let mut answers = (events.into_iter()).filter_map(|event| match event {
            SoundEvent::Separated {
                request: of,
                project,
                result,
                ..
            } if of == request => {
                assert_eq!(project, Uuid::from_u128(7));
                Some(result)
            }
            _ => None,
        });
        let answer = answers.next().expect("no answer");
        assert!(answers.next().is_none(), "two answers");
        answer
    }

    #[test]
    fn a_separator_answers_its_probe_by_naming_two_stems() {
        let dir = tempfile::tempdir().unwrap();
        let tool = separator(dir.path(), "demucs", "exit 0");
        let found = Separator::probe(&tool).unwrap();
        assert_eq!(found.program(), tool.as_os_str());
        assert_eq!(found.name(), "demucs");
        // A tool that ends badly, or that cannot split two stems, is none.
        // Its last line is the reason, not the whole of what it said.
        let broken = script(
            dir.path(),
            "broken",
            "echo 'Traceback (most recent call last):' >&2\necho \"ModuleNotFoundError: No module named 'numpy'\" >&2\nexit 2",
        );
        let said = Separator::probe(&broken).unwrap_err().to_string();
        assert!(
            said.starts_with("broken could not show its options"),
            "{said}"
        );
        assert!(
            said.ends_with(": ModuleNotFoundError: No module named 'numpy'"),
            "{said}"
        );
        assert!(
            !said.contains("Traceback") && !separator_absent(&said),
            "{said}"
        );
        let other = script(dir.path(), "other", "echo 'usage: other [-x]'");
        let said = Separator::probe(&other).unwrap_err().to_string();
        assert!(said.contains("--two-stems"), "{said}");
    }

    #[test]
    fn stems_are_put_in_the_media_folder_probed_and_said_in_order() {
        let _heavy = crate::heavy_test();
        if !tools_available() {
            eprintln!("SKIP: ffprobe is unavailable; the stems were not probed");
            return;
        }
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", WRITES);
        // The folder is not there yet: the job makes it.
        assert!(!bench.to.exists());
        let sent = bench.service.separate(bench.job(&tool)).unwrap();
        assert!(sent.replaced.is_empty());
        let events = bench.events();
        let progress: Vec<f32> = (events.iter())
            .filter_map(|event| match event {
                SoundEvent::Progress { request, done } if *request == sent.request => Some(*done),
                _ => None,
            })
            .collect();
        assert_eq!(progress, [0.5], "the tool's own figure, said once");
        let stems = separated(events, sent.request).unwrap();
        let paths: Vec<&Path> = stems.iter().map(|asset| asset.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                bench.to.join("song (Vocals).wav"),
                bench.to.join("song (Instruments).wav")
            ]
        );
        for stem in &stems {
            assert_eq!(stem.kind, MediaKind::Audio);
            assert!(stem.has_audio);
            assert!(
                (stem.duration.as_seconds() - 0.5).abs() < 0.01,
                "{:?}",
                stem.duration
            );
            assert_eq!(
                fs::read(&stem.path).unwrap(),
                fs::read(&bench.asset.path).unwrap()
            );
        }
        // The part folder went with what the tool left in it.
        assert_eq!(
            bench.kept(),
            ["song (Instruments).wav", "song (Vocals).wav"]
        );
    }

    #[test]
    fn a_stem_that_is_there_is_never_replaced() {
        if !tools_available() {
            eprintln!("SKIP: ffprobe is unavailable; the stems were not probed");
            return;
        }
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", WRITES);
        fs::create_dir_all(&bench.to).unwrap();
        fs::write(bench.to.join("song (Vocals).wav"), b"mine").unwrap();
        // A dangling link holds its name too.
        std::os::unix::fs::symlink(
            bench.dir.path().join("gone"),
            bench.to.join("song (Vocals) 2.wav"),
        )
        .unwrap();
        let sent = bench.service.separate(bench.job(&tool)).unwrap();
        let stems = separated(bench.events(), sent.request).unwrap();
        assert_eq!(stems[0].path, bench.to.join("song (Vocals) 3.wav"));
        assert_eq!(stems[1].path, bench.to.join("song (Instruments).wav"));
        assert_eq!(
            fs::read(bench.to.join("song (Vocals).wav")).unwrap(),
            b"mine"
        );
        assert!(
            fs::symlink_metadata(bench.to.join("song (Vocals) 2.wav"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_separation_given_up_or_replaced_leaves_neither_stems_nor_its_part_folder() {
        let _heavy = crate::heavy_test();
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", RUNS_ON);
        let job = bench.job(&tool);
        let part = job.part_folder();
        let begun = bench.to.join("begun");
        let wait_begun = || {
            let until = Instant::now() + Duration::from_secs(10);
            while !begun.exists() {
                assert!(Instant::now() < until, "the tool did not begin");
                thread::sleep(Duration::from_millis(5));
            }
            fs::remove_file(&begun).unwrap();
        };

        // Given up by name while the tool runs.
        let first = bench.service.separate(job.clone()).unwrap();
        wait_begun();
        assert!(part.join("htdemucs/vocals.wav").exists());
        assert_eq!(bench.service.cancel(first.request), Cancel::GivenUp);
        let events = bench.events();
        assert!(
            !(events.iter()).any(|event| matches!(event, SoundEvent::Separated { .. })),
            "{events:?}"
        );
        assert!(!part.exists());
        assert!(bench.kept().is_empty(), "{:?}", bench.kept());

        // Replaced by a newer separation of the same media, which is given
        // up in its turn.
        let older = bench.service.separate(job.clone()).unwrap();
        wait_begun();
        let newer = bench.service.separate(job).unwrap();
        assert_eq!(newer.replaced, [older.request]);
        wait_begun();
        bench.service.cancel(newer.request);
        assert!(bench.events().is_empty());
        assert!(!part.exists());
        assert!(bench.kept().is_empty(), "{:?}", bench.kept());
    }

    #[test]
    fn a_separation_outlives_a_project_replace() {
        if !tools_available() {
            eprintln!("SKIP: ffprobe is unavailable; the stems were not probed");
            return;
        }
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", WRITES);
        let sent = bench.service.separate(bench.job(&tool)).unwrap();
        bench.service.cancel_all();
        let stems = separated(bench.events(), sent.request).unwrap();
        assert_eq!(stems.len(), 2);
    }

    #[test]
    fn a_tool_that_fails_says_its_last_words_and_leaves_nothing() {
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", FAILS);
        let job = bench.job(&tool);
        let part = job.part_folder();
        let sent = bench.service.separate(job).unwrap();
        let said = separated(bench.events(), sent.request).unwrap_err();
        assert!(
            said.starts_with("demucs could not separate the sound"),
            "{said}"
        );
        assert!(said.contains('3'), "{said}");
        assert!(said.ends_with("RuntimeError: out of memory"), "{said}");
        assert!(!part.exists());
        assert!(bench.kept().is_empty(), "{:?}", bench.kept());
    }

    #[test]
    fn a_tool_that_ends_well_without_its_stems_is_an_error() {
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", r#"mkdir -p "$out/htdemucs""#);
        let sent = bench.service.separate(bench.job(&tool)).unwrap();
        let said = separated(bench.events(), sent.request).unwrap_err();
        assert_eq!(said, "demucs ended without writing vocals.wav");
        assert!(bench.kept().is_empty(), "{:?}", bench.kept());
    }

    #[test]
    fn a_part_folder_in_the_way_is_refused_and_left_as_it_is() {
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", WRITES);
        let job = bench.job(&tool);
        let part = job.part_folder();
        fs::create_dir_all(&part).unwrap();
        fs::write(part.join("notes.txt"), b"mine").unwrap();
        let sent = bench.service.separate(job).unwrap();
        let said = separated(bench.events(), sent.request).unwrap_err();
        assert!(
            said.ends_with(
                "is in the way: something of that name is already there, and it was left as it is"
            ),
            "{said}"
        );
        assert_eq!(fs::read(part.join("notes.txt")).unwrap(), b"mine");
    }

    #[test]
    fn a_missing_separator_found_after_its_probe_is_said_when_run() {
        let bench = bench();
        let tool = separator(bench.dir.path(), "demucs", "exit 0");
        let job = bench.job(&tool);
        fs::remove_file(&tool).unwrap();
        let sent = bench.service.separate(job.clone()).unwrap();
        let said = separated(bench.events(), sent.request).unwrap_err();
        assert!(said.starts_with("No stem separator was found"), "{said}");
        assert!(!job.part_folder().exists());
    }
}

// ---- With the person's own separator --------------------------------------------

/// The separator `NEPTUNE_VIDEO_SEPARATOR_TEST` names, over twenty seconds of two
/// tones FFmpeg makes, through the lane. Set it on a host with Demucs to
/// run this: it takes the tool's time, and its first run fetches the tool's
/// model. It says the tool's time and what it wrote, and claims nothing of
/// quality. Without it, it says it was skipped, as tests do without FFmpeg.
#[test]
fn the_persons_separator_splits_twenty_seconds_into_two_stems() {
    let Some(program) = std::env::var_os("NEPTUNE_VIDEO_SEPARATOR_TEST") else {
        eprintln!("SKIP: NEPTUNE_VIDEO_SEPARATOR_TEST names no separator; none was run");
        return;
    };
    let _heavy = crate::heavy_test();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tones.wav");
    let made = tool_command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
        ])
        .args(["-i", "sine=frequency=220:duration=20", "-f", "lavfi"])
        .args(["-i", "sine=frequency=1760:duration=20"])
        .args([
            "-filter_complex",
            "amix=inputs=2,aformat=channel_layouts=stereo",
        ])
        .args(["-ar", "44100"])
        .arg(&source)
        .status()
        .unwrap();
    assert!(made.success());
    let mut asset = Asset::new(source, MediaKind::Audio, Time::from_seconds(20.0));
    asset.has_audio = true;
    let probing = Instant::now();
    let separator = Separator::probe(program).unwrap();
    eprintln!("probe: {:.2} s", probing.elapsed().as_secs_f64());
    let service = SoundService::new(|| {}).unwrap();
    let to = dir.path().join("Film media");
    let job = SeparateJob {
        project: Uuid::nil(),
        asset,
        to: to.clone(),
        separator,
    };
    let part = job.part_folder();
    let started = Instant::now();
    let sent = service.separate(job).unwrap();
    let (mut said, mut answer) = (Vec::new(), None);
    while answer.is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(20 * 60),
            "no answer"
        );
        match service.poll() {
            Some(SoundEvent::Progress { done, .. }) => said.push(done),
            Some(SoundEvent::Separated {
                request, result, ..
            }) if request == sent.request => answer = Some(result),
            Some(_) => {}
            None => thread::sleep(Duration::from_millis(20)),
        }
    }
    let ran = started.elapsed().as_secs_f64();
    let stems = answer.unwrap().unwrap();
    eprintln!("separated in {ran:.2} s; progress said {said:?}");
    for stem in &stems {
        eprintln!(
            "{}: {:.3} s, {} bytes",
            stem.path.display(),
            stem.duration.as_seconds(),
            stem.file_size
        );
        assert!((stem.duration.as_seconds() - 20.0).abs() < 0.05);
    }
    assert_eq!(stems.len(), 2);
    assert!(!part.exists());
}
