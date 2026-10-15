//! Blocking media inspection for callers that own a worker thread: an
//! import without the import worker, every stream ffprobe reports, and the
//! places where the picture cuts to a new shot. Each call owns the processes
//! it starts and kills them when its cancel flag is raised, and FFmpeg reads
//! only local files.
use crate::{
    MediaError, io,
    process::{Process, Processes, capture, tool_command},
};
use editor_core::{Asset, MediaKind, Time};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub use crate::import::Imported;

/// The most ffprobe may write about one file.
pub const MAX_DETAILS_BYTES: usize = 4 * 1024 * 1024;
/// The most cuts one scene search returns; a search that finds more keeps
/// the strongest.
pub const MAX_SCENE_CUTS: usize = 10_000;
/// The width the picture is scaled to before shots are compared.
pub const SCENE_SIDE: u32 = 192;
/// A scene search whose FFmpeg writes nothing for this long is stopped.
pub const SCENE_STALL: Duration = Duration::from_secs(30);

/// Imports `paths` as the import worker does (folders walked, at most
/// 10,000 entries, every file probed), on the calling thread. Raising
/// `cancel` stops it between files and kills the probe under way.
pub fn import_now(
    paths: Vec<PathBuf>,
    cancel: &AtomicBool,
    allowed: &dyn Fn(&Path) -> bool,
) -> Imported {
    let processes = Processes::default();
    with_cancel(cancel, &processes, || {
        crate::import::import_paths_within(
            paths,
            &processes,
            || cancel.load(Ordering::Acquire),
            allowed,
        )
    })
}

/// Whether `path` has the extension of media the editor imports.
pub fn importable(path: &Path) -> bool {
    crate::import::supported(path)
}

/// Every format, stream and chapter field ffprobe reports for `path`, as
/// ffprobe's own JSON.
pub fn probe_details(path: &Path) -> Result<serde_json::Value, MediaError> {
    let mut command = tool_command("ffprobe");
    command
        .args([
            "-protocol_whitelist",
            "file",
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-show_chapters",
            "-of",
            "json",
        ])
        .arg(path);
    let bytes = capture(
        "ffprobe",
        &mut command,
        &Processes::default(),
        MAX_DETAILS_BYTES,
    )?;
    let details: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| MediaError::Decode(format!("ffprobe wrote unreadable JSON: {error}")))?;
    if let Some(format) = details["format"]["format_name"].as_str()
        && crate::import::is_reference_format(format)
    {
        return Err(MediaError::Invalid(format!(
            "This is a {format} file, which refers to other files rather than holding media"
        )));
    }
    Ok(details)
}

/// A place where the picture changes to another shot.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct SceneCut {
    /// Asset time of the first frame of the new shot.
    pub at: Time,
    /// How different the frame is from the one before, 0 to 1.
    pub score: f32,
}

/// Finds the cuts between shots in `asset` from `from` to `to` (asset
/// time): the frames whose difference from the frame before, as FFmpeg's
/// scene score measures it on a picture [`SCENE_SIDE`] wide, is at least
/// `threshold` (0 to 1; 0.3 to 0.4 suits ordinary cuts). `progress` is told
/// the fraction done. Raising `cancel` kills FFmpeg and answers `Cancelled`.
pub fn scene_cuts(
    asset: &Asset,
    from: Time,
    to: Time,
    threshold: f32,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Vec<SceneCut>, MediaError> {
    if asset.kind != MediaKind::Video {
        return Err(MediaError::Invalid(format!(
            "{} has no moving picture to find shots in",
            asset.name
        )));
    }
    if !(threshold.is_finite() && (0.0..=1.0).contains(&threshold)) {
        return Err(MediaError::Invalid(
            "The scene threshold is a number from 0 to 1".into(),
        ));
    }
    let to = to.min(asset.duration);
    if from.0 < 0 || from >= to {
        return Err(MediaError::Invalid(
            "The range to search is empty or outside the media".into(),
        ));
    }
    let span = to - from;
    let mut command = tool_command("ffmpeg");
    command
        .args(["-hide_banner", "-nostdin", "-nostats", "-v", "error"])
        .args(["-protocol_whitelist", "file"])
        .args(["-ss", &seconds(from), "-t", &seconds(span), "-i"])
        .arg(&asset.path)
        .args(["-map", "0:V:0", "-an", "-sn", "-dn", "-vf"])
        .arg(format!(
            "scale={SCENE_SIDE}:-2:flags=fast_bilinear,select='gte(scene,0)',metadata=mode=print:key=lavfi.scene_score:file=-"
        ))
        .args(["-f", "null", "-"]);
    let processes = Processes::default();
    with_cancel(cancel, &processes, || {
        read_cuts(
            &mut command,
            &processes,
            from,
            span,
            threshold,
            cancel,
            progress,
        )
    })
}

fn read_cuts(
    command: &mut std::process::Command,
    processes: &Processes,
    from: Time,
    span: Time,
    threshold: f32,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Vec<SceneCut>, MediaError> {
    let mut process = Process::spawn("ffmpeg", command, processes)?;
    let mut cuts = Vec::new();
    let mut at = None;
    let mut first = true;
    let mut reported = Instant::now();
    let mut line = String::new();
    // The watchdog kills FFmpeg when a line takes longer than SCENE_STALL.
    process.begin_read_until(Instant::now() + SCENE_STALL);
    let mut reader = BufReader::new(&mut process.stdout);
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| io("Cannot read FFmpeg's scene scores", error))?;
        if read == 0 {
            break;
        }
        if cancel.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        if let Some(time) = line.split_whitespace().find_map(|part| {
            part.strip_prefix("pts_time:")
                .and_then(|text| text.parse::<f64>().ok())
        }) {
            at = Some(time);
            if reported.elapsed() >= Duration::from_millis(200) && span.0 > 0 {
                reported = Instant::now();
                progress(
                    (Time::from_seconds(time).0 as f64 / span.0 as f64).clamp(0.0, 1.0) as f32,
                );
            }
        } else if let Some(score) = line
            .trim()
            .strip_prefix("lavfi.scene_score=")
            .and_then(|text| text.parse::<f32>().ok())
            && let Some(time) = at.take()
        {
            // The first frame has nothing before it to differ from.
            if !std::mem::take(&mut first) && score >= threshold {
                cuts.push(SceneCut {
                    at: from + Time::from_seconds(time),
                    score,
                });
            }
        }
    }
    drop(reader);
    if cancel.load(Ordering::Acquire) {
        return Err(MediaError::Cancelled);
    }
    let finished = process.end_read_checked();
    let until = Instant::now() + Duration::from_secs(10);
    let status = loop {
        let waited = process
            .child
            .lock()
            .map_err(|_| MediaError::Closed)?
            .try_wait()
            .map_err(|error| io("Cannot wait for FFmpeg", error))?;
        if let Some(status) = waited {
            break status;
        }
        if Instant::now() >= until {
            return Err(process.failure("did not finish"));
        }
        thread::sleep(Duration::from_millis(5));
    };
    if !finished || !status.success() {
        return Err(process.failure("stopped before the end of the picture"));
    }
    if cuts.len() > MAX_SCENE_CUTS {
        cuts.sort_by(|a, b| b.score.total_cmp(&a.score));
        cuts.truncate(MAX_SCENE_CUTS);
        cuts.sort_by_key(|cut| cut.at);
    }
    progress(1.0);
    Ok(cuts)
}

fn seconds(time: Time) -> String {
    format!("{:.6}", time.as_seconds())
}

/// Runs `work` while a scoped thread kills `processes` once `cancel` is
/// raised, so a blocking read inside `work` is ended by the kill.
fn with_cancel<T>(cancel: &AtomicBool, processes: &Processes, work: impl FnOnce() -> T) -> T {
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if cancel.load(Ordering::Acquire) {
                    processes.kill_all();
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        let result = work();
        done.store(true, Ordering::Release);
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two seconds of a test pattern, then two of flat red: one hard cut at
    /// frame 60 of 30 fps.
    fn cut_video(dir: &Path) -> PathBuf {
        let path = dir.join("cut.mp4");
        let status = tool_command("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x180:rate=30:duration=2")
            .args([
                "-f",
                "lavfi",
                "-i",
                "color=c=red:size=320x180:rate=30:duration=2",
            ])
            .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    fn imported(path: PathBuf) -> Asset {
        let mut imported = import_now(vec![path], &AtomicBool::new(false), &|_| true);
        assert!(imported.errors.is_empty(), "{:?}", imported.errors);
        imported.assets.remove(0)
    }

    #[test]
    fn a_hard_cut_is_found_at_its_frame_and_nowhere_else() {
        let dir = tempfile::tempdir().unwrap();
        let asset = imported(cut_video(dir.path()));
        let cuts = scene_cuts(
            &asset,
            Time::ZERO,
            asset.duration,
            0.35,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        assert!(
            (cuts[0].at.as_seconds() - 2.0).abs() < 1.0 / 30.0,
            "{:?}",
            cuts[0]
        );
        // A search after the cut finds nothing.
        let later = Time::from_seconds(2.5);
        assert!(
            scene_cuts(
                &asset,
                later,
                asset.duration,
                0.35,
                &AtomicBool::new(false),
                &|_| {}
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn a_search_is_refused_outside_the_media_or_with_a_threshold_past_one() {
        let dir = tempfile::tempdir().unwrap();
        let asset = imported(cut_video(dir.path()));
        let never = AtomicBool::new(false);
        assert!(
            scene_cuts(
                &asset,
                asset.duration,
                asset.duration,
                0.35,
                &never,
                &|_| {}
            )
            .is_err()
        );
        assert!(scene_cuts(&asset, Time::ZERO, asset.duration, 1.5, &never, &|_| {}).is_err());
        let cancelled = AtomicBool::new(true);
        assert!(
            scene_cuts(
                &asset,
                Time::ZERO,
                asset.duration,
                0.35,
                &cancelled,
                &|_| {}
            )
            .is_err()
        );
    }

    #[test]
    fn a_playlist_is_never_media_whatever_it_lists() {
        let dir = tempfile::tempdir().unwrap();
        let video = cut_video(dir.path());
        let list = dir.path().join("list.txt");
        std::fs::write(&list, "ffconcat version 1.0\nfile cut.mp4\n").unwrap();
        // Named as media, it is still read as a list by FFmpeg's own sniffing.
        let named = dir.path().join("list.mp4");
        std::fs::copy(&list, &named).unwrap();
        let imported = import_now(vec![named.clone()], &AtomicBool::new(false), &|_| true);
        assert!(imported.assets.is_empty(), "{:?}", imported.assets);
        assert!(probe_details(&named).is_err());
        assert!(probe_details(&video).is_ok());
    }

    #[test]
    fn an_import_reads_nothing_its_predicate_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let video = cut_video(dir.path());
        let imported = import_now(
            vec![dir.path().to_path_buf()],
            &AtomicBool::new(false),
            &|path| path != video.canonicalize().unwrap(),
        );
        assert!(imported.assets.is_empty());
        assert_eq!(imported.errors.len(), 1);
        assert!(imported.errors[0].message.contains("outside"));
    }

    #[test]
    fn details_name_every_stream_the_file_has() {
        let dir = tempfile::tempdir().unwrap();
        let details = probe_details(&cut_video(dir.path())).unwrap();
        let streams = details["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0]["codec_type"], "video");
        assert_eq!(streams[0]["width"], 320);
        assert!(details["format"]["duration"].is_string());
    }
}
