//! The lane's queue and cache with a runner that writes bytes and waits to
//! be told, and the whole of it on files FFmpeg makes.
use super::*;
use crate::import::asset_from_probe;
use std::{
    process::Stdio,
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};

const LIMIT: Duration = Duration::from_secs(60);

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + LIMIT;
    while !done() {
        assert!(Instant::now() < limit, "{what} did not happen in time");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A video asset of a file that exists, as import would describe it.
fn video(path: &Path, width: u32, height: u32) -> Asset {
    if !path.exists() {
        fs::write(path, path.to_string_lossy().as_bytes()).unwrap();
    }
    let mut asset = Asset::new(path.to_owned(), MediaKind::Video, Time::from_seconds(1.0));
    asset.width = width;
    asset.height = height;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = fs::metadata(path).unwrap().len();
    asset
}

/// Lets a job of the fake runner end, or keeps it waiting.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

/// A lane whose runner writes `bytes` of nothing as the proxy, once its
/// gate is open, and counts how often it was started and cancelled.
struct Fake {
    service: ProxyService,
    gate: Arc<Gate>,
    runs: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    dir: PathBuf,
}

fn fake(dir: &Path, budget: u64, bytes: usize, open: bool) -> Fake {
    let gate = Arc::new(Gate::default());
    if open {
        gate.open();
    }
    let (runs, cancelled) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (held, started, stopped) = (Arc::clone(&gate), Arc::clone(&runs), Arc::clone(&cancelled));
    let service = ProxyService::with_runner(
        Arc::new(|| {}),
        dir.to_owned(),
        budget,
        Processes::default(),
        Arc::new(move |job: &Encode, control: &Control| {
            started.fetch_add(1, Ordering::AcqRel);
            fs::write(&job.part, vec![0u8; bytes]).unwrap();
            control.progress(0.5);
            let mut open = held.open.lock().unwrap();
            while !*open {
                if control.cancelled() {
                    stopped.fetch_add(1, Ordering::AcqRel);
                    return Err(MediaError::Cancelled);
                }
                open = (held.changed)
                    .wait_timeout(open, Duration::from_millis(5))
                    .unwrap()
                    .0;
            }
            Ok(Encoded {
                codec: Codec::H264,
                frames: job.frames,
            })
        }),
    )
    .unwrap();
    until("the cache's directory being read", || service.loaded());
    Fake {
        service,
        gate,
        runs,
        cancelled,
        dir: dir.to_owned(),
    }
}

impl Fake {
    fn ready(&self, asset: &Asset) -> Proxy {
        let mut found = None;
        until("a proxy", || {
            found = match self.service.lookup(asset) {
                ProxyStatus::Ready(proxy) => Some(proxy),
                ProxyStatus::Failed(message) => panic!("the proxy failed: {message}"),
                _ => None,
            };
            found.is_some()
        });
        found.unwrap()
    }

    fn files(&self, suffix: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = (fs::read_dir(&self.dir).into_iter().flatten().flatten())
            .map(|item| item.path())
            .filter(|path| path.to_string_lossy().ends_with(suffix))
            .collect();
        found.sort();
        found
    }

    fn events(&self) -> Vec<ProxyEvent> {
        std::iter::from_fn(|| self.service.poll()).collect()
    }
}

#[test]
fn large_or_dense_videos_want_proxies() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("clip.mov");
    let mut asset = video(&path, 3840, 2160);
    assert!(wants_proxy(&asset));
    asset.width = 1080;
    asset.height = 1921;
    assert!(
        wants_proxy(&asset),
        "a tall video is measured on its long side"
    );
    asset.height = 1920;
    asset.file_size = 6_000_000;
    assert!(!wants_proxy(&asset), "48 Mbit/s over a second");
    asset.file_size = 6_250_001;
    assert!(wants_proxy(&asset), "just over 50 Mbit/s");
    for kind in [MediaKind::Image, MediaKind::Gif, MediaKind::Audio] {
        let mut other = video(&path, 8000, 6000);
        other.kind = kind;
        assert!(!wants_proxy(&other), "{kind:?} has no proxy");
    }
}

#[test]
fn a_proxy_keeps_the_aspect_within_its_long_side_on_even_sides() {
    assert_eq!(proxy_size(2560, 1440), [1280, 720]);
    assert_eq!(proxy_size(3840, 2160), [1280, 720]);
    assert_eq!(proxy_size(1080, 1920), [720, 1280]);
    assert_eq!(proxy_size(4096, 2160), [1280, 676]);
    assert_eq!(proxy_size(640, 360), [640, 360], "never larger");
    assert_eq!(proxy_size(1279, 721), [1280, 722]);
    assert_eq!(proxy_size(1, 1), [2, 2]);
    for (width, height) in [(7680, 4320), (4000, 3000), (720, 1280), (2048, 858)] {
        let [w, h] = proxy_size(width, height);
        assert!(w.max(h) <= PROXY_SIDE && w % 2 == 0 && h % 2 == 0);
        let (was, is) = (width as f64 / height as f64, w as f64 / h as f64);
        assert!(
            (was - is).abs() / was < 0.01,
            "{width}x{height} became {w}x{h}"
        );
    }
}

#[test]
fn the_key_is_of_the_source_on_disk_and_the_profile() {
    let temp = tempfile::tempdir().unwrap();
    let asset = video(&temp.path().join("a.mp4"), 3840, 2160);
    let source = SourceIdentity::of(&asset);
    let stamp = Stamp::read(&asset.path).unwrap();
    let base = key(&source, &stamp);
    assert!(is_key(&base));
    assert_eq!(base, key(&source, &Stamp::read(&asset.path).unwrap()));
    let changed = [
        key(
            &source,
            &Stamp {
                length: stamp.length + 1,
                ..stamp.clone()
            },
        ),
        key(
            &source,
            &Stamp {
                modified: stamp.modified + 1,
                ..stamp.clone()
            },
        ),
        key(
            &source,
            &Stamp {
                canonical: temp.path().join("b.mp4"),
                ..stamp.clone()
            },
        ),
        key(
            &SourceIdentity {
                file_size: 1,
                ..source.clone()
            },
            &stamp,
        ),
        key(
            &SourceIdentity {
                width: 1920,
                ..source.clone()
            },
            &stamp,
        ),
        key(
            &SourceIdentity {
                duration: Time(2),
                ..source.clone()
            },
            &stamp,
        ),
        key(
            &SourceIdentity {
                frame_rate: None,
                ..source.clone()
            },
            &stamp,
        ),
    ];
    for other in changed {
        assert_ne!(other, base);
    }
}

#[test]
fn only_a_video_with_a_picture_has_a_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), PROXY_BUDGET, 16, true);
    let mut asset = video(&temp.path().join("a.mp4"), 3840, 2160);
    asset.kind = MediaKind::Image;
    assert!(matches!(
        lane.service.request(&asset),
        Err(MediaError::Invalid(_))
    ));
    asset.kind = MediaKind::Video;
    asset.width = 0;
    assert!(matches!(
        lane.service.request(&asset),
        Err(MediaError::Invalid(_))
    ));
    assert_eq!(lane.runs.load(Ordering::Acquire), 0);
}

#[test]
fn a_proxy_is_asked_for_once_and_again_for_a_changed_source() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), PROXY_BUDGET, 100, false);
    let asset = video(&temp.path().join("a.mp4"), 3840, 2160);
    let other = video(&temp.path().join("b.mp4"), 3840, 2160);
    lane.service.request(&asset).unwrap();
    until("the job starting", || {
        lane.runs.load(Ordering::Acquire) == 1
    });
    until("its progress", || {
        lane.service.lookup(&asset) == ProxyStatus::Generating(0.5)
    });
    // Asked again while it is written, and while another waits.
    lane.service.request(&asset).unwrap();
    lane.service.request(&other).unwrap();
    lane.service.request(&other).unwrap();
    assert_eq!(lane.service.lookup(&other), ProxyStatus::Queued);
    // The same asset is now another file: the old one is given up.
    let mut changed = asset.clone();
    changed.file_size += 1;
    lane.service.request(&changed).unwrap();
    until("the old job being stopped and the next started", || {
        lane.cancelled.load(Ordering::Acquire) == 1 && lane.runs.load(Ordering::Acquire) == 2
    });
    assert_eq!(lane.service.lookup(&asset), ProxyStatus::None);
    lane.gate.open();
    // `other` was asked first, then the changed asset: two more jobs.
    until("both written", || !lane.service.busy());
    assert_eq!(lane.runs.load(Ordering::Acquire), 3);
    let events = lane.events();
    let ready: Vec<AssetId> = (events.iter())
        .filter_map(|event| match event {
            ProxyEvent::Ready { asset, .. } => Some(*asset),
            _ => None,
        })
        .collect();
    assert_eq!(ready, [other.id, asset.id]);
    assert!(
        (events.iter()).all(|event| match event {
            ProxyEvent::Ready { proxy, .. } => proxy.source != SourceIdentity::of(&asset),
            ProxyEvent::Progress { source, .. } | ProxyEvent::Failed { source, .. } =>
                *source != SourceIdentity::of(&asset),
            ProxyEvent::Removed { .. } => true,
        }),
        "the replaced job said nothing"
    );
    assert!(matches!(
        lane.service.lookup(&changed),
        ProxyStatus::Ready(_)
    ));
    assert_eq!(lane.service.lookup(&asset), ProxyStatus::None);
    assert_eq!(lane.service.usage(), (200, 2));
    // A proxy the cache has is checked, not written again.
    lane.service.request(&other).unwrap();
    until("the check", || !lane.service.busy());
    assert_eq!(lane.runs.load(Ordering::Acquire), 3);
    assert!(matches!(
        lane.events().as_slice(),
        [ProxyEvent::Ready { asset, .. }] if *asset == other.id
    ));
    assert!(lane.files(".part").is_empty());
}

#[test]
fn a_full_queue_is_busy() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), PROXY_BUDGET, 1, false);
    let first = video(&temp.path().join("first.mp4"), 3840, 2160);
    lane.service.request(&first).unwrap();
    until("the job starting", || {
        lane.runs.load(Ordering::Acquire) == 1
    });
    let waiting: Vec<Asset> = (0..PROXY_QUEUE)
        .map(|index| video(&temp.path().join(format!("{index}.mp4")), 3840, 2160))
        .collect();
    for asset in &waiting {
        lane.service.request(asset).unwrap();
    }
    let more = video(&temp.path().join("more.mp4"), 3840, 2160);
    assert!(matches!(lane.service.request(&more), Err(MediaError::Busy)));
    // Asking again for one that waits is no more work.
    lane.service.request(&waiting[3]).unwrap();
    lane.service.cancel(waiting[0].id);
    lane.service.request(&more).unwrap();
    lane.service.cancel_all();
    until("the running job being stopped", || {
        lane.cancelled.load(Ordering::Acquire) == 1
    });
    until("the lane being idle", || !lane.service.busy());
    assert_eq!(lane.runs.load(Ordering::Acquire), 1);
    assert!(lane.events().is_empty());
    assert!(lane.files(".part").is_empty());
}

#[test]
fn a_cancelled_proxy_says_nothing_and_leaves_no_file() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), PROXY_BUDGET, 1000, false);
    let asset = video(&temp.path().join("a.mp4"), 3840, 2160);
    lane.service.request(&asset).unwrap();
    until("the job starting", || {
        lane.runs.load(Ordering::Acquire) == 1
    });
    until("its part file", || lane.files(".part").len() == 1);
    lane.service.cancel(asset.id);
    until("the job being stopped", || !lane.service.busy());
    assert_eq!(lane.cancelled.load(Ordering::Acquire), 1);
    assert!(lane.events().is_empty());
    assert_eq!(lane.service.lookup(&asset), ProxyStatus::None);
    assert!(lane.files(".part").is_empty());
    assert_eq!(lane.service.usage(), (0, 0));
}

#[test]
fn the_budget_removes_the_least_recently_used_but_not_an_open_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), 250, 100, true);
    let assets: Vec<Asset> = (0..4)
        .map(|index| video(&temp.path().join(format!("{index}.mp4")), 3840, 2160))
        .collect();
    let mut proxies = Vec::new();
    for asset in &assets[..2] {
        lane.service.request(asset).unwrap();
        proxies.push(lane.ready(asset));
    }
    // The older is open: making room for a third takes the second.
    lane.service
        .set_in_use(std::slice::from_ref(&proxies[0].path));
    lane.service.request(&assets[2]).unwrap();
    proxies.push(lane.ready(&assets[2]));
    until("room being made", || !proxies[1].path.exists());
    assert_eq!(lane.service.lookup(&assets[1]), ProxyStatus::None);
    assert!(proxies[0].path.exists() && proxies[2].path.exists());
    assert_eq!(lane.service.usage(), (200, 2));
    // Let go, the first is now the least recently used.
    lane.service.set_in_use(&[]);
    lane.service.request(&assets[3]).unwrap();
    lane.ready(&assets[3]);
    until("room being made", || !proxies[0].path.exists());
    assert!(proxies[2].path.exists());
    assert_eq!(lane.files(".json").len(), 2);
    // A smaller budget keeps the proxy written last, even over it.
    lane.service.set_budget(50);
    until("room being made", || !proxies[2].path.exists());
    assert_eq!(lane.service.usage(), (100, 1));
    assert!(matches!(
        lane.service.lookup(&assets[3]),
        ProxyStatus::Ready(_)
    ));
}

#[test]
fn the_budget_tells_of_the_proxies_it_removes() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), 250, 100, true);
    let assets: Vec<Asset> = (0..3)
        .map(|index| video(&temp.path().join(format!("{index}.mp4")), 3840, 2160))
        .collect();
    let mut proxies = Vec::new();
    for asset in &assets {
        lane.service.request(asset).unwrap();
        proxies.push(lane.ready(asset));
    }
    // The third made room by taking the first, the least recently used.
    until("room being made", || !proxies[0].path.exists());
    let mut removed = Vec::new();
    until("the removal being told", || {
        removed.extend(lane.events().into_iter().filter_map(|event| match event {
            ProxyEvent::Removed { proxies } => Some(proxies),
            _ => None,
        }));
        !removed.is_empty()
    });
    assert_eq!(removed, vec![vec![proxies[0].clone()]]);
    assert_eq!(lane.service.lookup(&assets[0]), ProxyStatus::None);
    // A smaller budget tells of what it takes too; the newest stays.
    lane.service.set_budget(50);
    until("room being made", || !proxies[1].path.exists());
    let mut removed = Vec::new();
    until("the removal being told", || {
        removed.extend(lane.events().into_iter().filter_map(|event| match event {
            ProxyEvent::Removed { proxies } => Some(proxies),
            _ => None,
        }));
        !removed.is_empty()
    });
    assert_eq!(removed, vec![vec![proxies[1].clone()]]);
    assert!(matches!(
        lane.service.lookup(&assets[2]),
        ProxyStatus::Ready(_)
    ));
    // Removing by hand and clearing are not told of: the caller did them.
    lane.service.remove(&assets[2]);
    lane.service.clear();
    until("the cache emptying", || lane.service.usage() == (0, 0));
    lane.service.set_budget(40);
    thread::sleep(Duration::from_millis(50));
    assert!(
        !(lane.events().iter()).any(|event| matches!(event, ProxyEvent::Removed { .. })),
        "only the budget's removals are told of"
    );
}

#[test]
fn clear_empties_the_cache_and_an_open_proxy_goes_when_let_go() {
    let temp = tempfile::tempdir().unwrap();
    let lane = fake(temp.path(), PROXY_BUDGET, 10, true);
    let (first, second) = (
        video(&temp.path().join("1.mp4"), 3840, 2160),
        video(&temp.path().join("2.mp4"), 3840, 2160),
    );
    lane.service.request(&first).unwrap();
    let open = lane.ready(&first);
    lane.service.request(&second).unwrap();
    let closed = lane.ready(&second);
    lane.service.set_in_use(std::slice::from_ref(&open.path));
    lane.service.clear();
    assert_eq!(lane.service.usage(), (0, 0));
    assert_eq!(lane.service.lookup(&first), ProxyStatus::None);
    until("the closed proxy being removed", || !closed.path.exists());
    assert!(
        lane.files(".json").is_empty(),
        "no session finds either again"
    );
    assert!(open.path.exists(), "an open proxy stays while it is read");
    lane.service.set_in_use(&[]);
    until("the open proxy being removed", || !open.path.exists());
    // `remove` is the same for one asset.
    lane.service.request(&first).unwrap();
    let again = lane.ready(&first);
    lane.service.remove(&first);
    assert_eq!(lane.service.lookup(&first), ProxyStatus::None);
    until("its removal", || !again.path.exists());
}

#[test]
fn a_kept_proxy_is_found_after_a_restart_unless_its_source_changed() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("proxies");
    let (kept, changed) = (
        video(&temp.path().join("kept.mp4"), 3840, 2160),
        video(&temp.path().join("changed.mp4"), 3840, 2160),
    );
    let proxies = {
        let lane = fake(&cache, PROXY_BUDGET, 64, true);
        lane.service.request(&kept).unwrap();
        lane.service.request(&changed).unwrap();
        [lane.ready(&kept), lane.ready(&changed)]
    };
    // The same length, written later: only the disk can tell.
    let bytes = fs::read(&changed.path).unwrap();
    thread::sleep(Duration::from_millis(20));
    fs::write(
        &changed.path,
        bytes.iter().map(|byte| !byte).collect::<Vec<_>>(),
    )
    .unwrap();
    let lane = fake(&cache, PROXY_BUDGET, 64, true);
    assert_eq!(
        lane.service.lookup(&kept),
        ProxyStatus::Ready(proxies[0].clone())
    );
    assert_eq!(lane.service.lookup(&changed), ProxyStatus::None);
    until("the stale proxy being removed", || {
        !proxies[1].path.exists()
    });
    assert_eq!(lane.service.usage(), (64, 1));
    assert_eq!(lane.files(".json").len(), 1);
    // Changed while a session runs, a request finds out and writes it anew.
    lane.service.request(&changed).unwrap();
    let anew = lane.ready(&changed);
    assert_ne!(anew.path, proxies[1].path);
    let bytes = fs::read(&kept.path).unwrap();
    fs::write(
        &kept.path,
        bytes.iter().map(|byte| !byte).collect::<Vec<_>>(),
    )
    .unwrap();
    lane.service.request(&kept).unwrap();
    until("the stale proxy being written anew", || {
        lane.runs.load(Ordering::Acquire) == 2 && !lane.service.busy()
    });
    let fresh = lane.ready(&kept);
    assert_ne!(fresh.path, proxies[0].path);
    until("the stale proxy being removed", || {
        !proxies[0].path.exists()
    });
    // A manifest that is damaged or of another profile is discarded.
    drop(lane);
    let manifest = fresh.path.with_extension("json");
    let mut text: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    text["profile"] = (PROXY_PROFILE + 1).into();
    fs::write(&manifest, serde_json::to_vec(&text).unwrap()).unwrap();
    fs::write(anew.path.with_extension("json"), b"{").unwrap();
    let lane = fake(&cache, PROXY_BUDGET, 64, true);
    assert_eq!(lane.service.usage(), (0, 0));
    until("both being removed", || lane.files("").is_empty());
}

#[test]
fn left_over_part_files_are_swept_once_an_hour_old() {
    let temp = tempfile::tempdir().unwrap();
    let name = "0".repeat(64);
    let old = temp.path().join(format!("{name}.1234.part"));
    let young = temp.path().join(format!("{name}.5678.part"));
    let orphan = temp.path().join(format!("{name}.mp4"));
    let foreign = temp.path().join("notes.part");
    let hours_ago = SystemTime::now() - Duration::from_secs(7200);
    for path in [&old, &young, &orphan, &foreign] {
        fs::write(path, b"part").unwrap();
    }
    for path in [&old, &orphan, &foreign] {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(hours_ago).unwrap();
    }
    let lane = fake(temp.path(), PROXY_BUDGET, 1, true);
    until("old files being swept", || {
        !old.exists() && !orphan.exists()
    });
    assert!(young.exists(), "another session may be writing it");
    assert!(foreign.exists(), "not the cache's");
    assert_eq!(lane.service.usage(), (0, 0));
}

// ---- FFmpeg ---------------------------------------------------------------

fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        tool_command(tool)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn ffmpeg(args: &[&str], output: &Path) {
    let status = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "FFmpeg could not write {}",
        output.display()
    );
}

/// The asset import makes of a file.
fn imported(path: &Path) -> Asset {
    let output = tool_command("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height,duration,avg_frame_rate,r_frame_rate:stream_disposition=attached_pic:stream_tags=rotate:stream_side_data=rotation", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut asset = asset_from_probe(path, &output.stdout).unwrap();
    asset.file_size = fs::metadata(path).unwrap().len();
    asset
}

/// What ffprobe finds in a proxy, its frames counted.
fn probed(path: &Path) -> serde_json::Value {
    let output = tool_command("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "stream=codec_type,codec_name,width,height,r_frame_rate,avg_frame_rate,nb_read_frames:stream_side_data=rotation:format=format_name",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A lane that runs FFmpeg, with the codec chosen when one is given and
/// the source read no faster than it plays when `paced`.
fn real(dir: &Path, codec: Option<Codec>, paced: bool) -> (ProxyService, Processes) {
    let processes = Processes::default();
    let children = processes.clone();
    let known = Arc::new(Mutex::new(codec));
    let service = ProxyService::with_runner(
        Arc::new(|| {}),
        dir.to_owned(),
        PROXY_BUDGET,
        processes.clone(),
        Arc::new(move |job, control| encode(job, &children, &known, paced, control)),
    )
    .unwrap();
    until("the cache's directory being read", || service.loaded());
    (service, processes)
}

fn answer(service: &ProxyService) -> (Vec<f32>, Result<Proxy, String>) {
    let limit = Instant::now() + Duration::from_secs(120);
    let mut progress = Vec::new();
    loop {
        assert!(Instant::now() < limit, "the proxy was not written");
        match service.poll() {
            Some(ProxyEvent::Progress { done, .. }) => progress.push(done),
            Some(ProxyEvent::Ready { proxy, .. }) => return (progress, Ok(proxy)),
            Some(ProxyEvent::Failed { message, .. }) => return (progress, Err(message)),
            Some(ProxyEvent::Removed { .. }) => {}
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

#[test]
fn a_proxy_has_the_originals_frames_smaller_and_no_sound() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the H.264 proxy fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("wide.mkv");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=2560x1440:r=30",
            "-f",
            "lavfi",
            "-i",
            "sine=r=48000",
            "-t",
            "1",
            "-c:v",
            "ffv1",
            "-c:a",
            "pcm_s16le",
        ],
        &source,
    );
    let asset = imported(&source);
    assert_eq!((asset.width, asset.height), (2560, 1440));
    assert!(wants_proxy(&asset));
    let cache = temp.path().join("proxies");
    let (service, processes) = real(&cache, None, false);
    service.request(&asset).unwrap();
    let (progress, proxy) = answer(&service);
    let proxy = proxy.unwrap();
    assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!((proxy.width, proxy.height), (1280, 720));
    assert_eq!(proxy.source, SourceIdentity::of(&asset));
    assert!(proxy.path.starts_with(&cache));
    let found = probed(&proxy.path);
    let streams = found["streams"].as_array().unwrap();
    assert_eq!(streams.len(), 1, "a picture and no sound: {found}");
    let stream = &streams[0];
    assert_eq!(stream["codec_type"], "video");
    // H.264 when this FFmpeg has libx264, as the configured one does.
    let codec = codec_here(&Processes::default(), &Mutex::new(None)).unwrap();
    let (name, format) = match codec {
        Codec::H264 => ("h264", "mp4"),
        Codec::Mjpeg => ("mjpeg", "mov"),
    };
    assert_eq!(stream["codec_name"], name);
    assert_eq!(
        (stream["width"].as_u64(), stream["height"].as_u64()),
        (Some(1280), Some(720))
    );
    assert_eq!(stream["r_frame_rate"], "30/1");
    assert_eq!(stream["avg_frame_rate"], "30/1");
    assert_eq!(stream["nb_read_frames"], "30", "the source's 30 frames");
    assert!(
        found["format"]["format_name"]
            .as_str()
            .unwrap()
            .contains(format)
    );
    assert_eq!(service.lookup(&asset), ProxyStatus::Ready(proxy.clone()));
    let (bytes, count) = service.usage();
    assert_eq!(
        (bytes, count),
        (fs::metadata(&proxy.path).unwrap().len(), 1)
    );
    // Another asset of the same file finds it without FFmpeg.
    let started = processes.started_tools();
    let mut twin = asset.clone();
    twin.id = Uuid::new_v4();
    service.request(&twin).unwrap();
    let (_, again) = answer(&service);
    assert_eq!(again.unwrap(), proxy);
    assert_eq!(processes.started_tools(), started);
    drop(service);
    // The next session finds it from its manifest.
    let (service, _) = real(&cache, None, false);
    assert_eq!(service.lookup(&asset), ProxyStatus::Ready(proxy));
}

#[test]
fn without_libx264_a_proxy_is_mjpeg() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the MJPEG proxy fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("tall.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1080x1920:r=24000/1001",
            "-t",
            "0.5",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
        ],
        &source,
    );
    let asset = imported(&source);
    let (service, _) = real(&temp.path().join("proxies"), Some(Codec::Mjpeg), false);
    service.request(&asset).unwrap();
    let proxy = answer(&service).1.unwrap();
    assert_eq!((proxy.width, proxy.height), (720, 1280));
    assert_eq!(proxy.path.extension().unwrap(), "mov");
    let found = probed(&proxy.path);
    let stream = &found["streams"][0];
    assert_eq!(found["streams"].as_array().unwrap().len(), 1);
    assert_eq!(stream["codec_name"], "mjpeg");
    assert_eq!(stream["r_frame_rate"], "24000/1001");
    let rate = asset.frame_rate.unwrap();
    let frames = rate.frame_at(Time(asset.duration.0 - 1)) + 1;
    assert_eq!(stream["nb_read_frames"], frames.to_string().as_str());
}

#[test]
fn a_rotated_video_has_an_upright_proxy() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the rotated proxy fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("plain.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30",
            "-t",
            "0.5",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
        ],
        &plain,
    );
    let rotated = temp.path().join("rotated.mp4");
    let status = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(["-display_rotation", "90", "-i"])
        .arg(&plain)
        .args(["-c", "copy"])
        .arg(&rotated)
        .status()
        .unwrap();
    assert!(status.success());
    let asset = imported(&rotated);
    assert_eq!(
        (asset.width, asset.height),
        (360, 640),
        "import turns it upright"
    );
    let (service, _) = real(&temp.path().join("proxies"), None, false);
    service.request(&asset).unwrap();
    let proxy = answer(&service).1.unwrap();
    assert_eq!((proxy.width, proxy.height), (360, 640));
    let found = probed(&proxy.path);
    let stream = &found["streams"][0];
    assert_eq!(
        (stream["width"].as_u64(), stream["height"].as_u64()),
        (Some(360), Some(640))
    );
    assert!(
        stream.get("side_data_list").is_none(),
        "no rotation is left to apply: {found}"
    );
    assert_eq!(stream["nb_read_frames"], "15");
}

#[test]
fn cancel_kills_the_encoder_and_removes_its_part_file() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the proxy cancellation fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("long.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30",
            "-t",
            "20",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
        ],
        &source,
    );
    let asset = imported(&source);
    let cache = temp.path().join("proxies");
    // Read no faster than it plays: twenty seconds to write.
    let (service, processes) = real(&cache, None, true);
    service.request(&asset).unwrap();
    until(
        "progress",
        || matches!(service.lookup(&asset), ProxyStatus::Generating(done) if done > 0.0),
    );
    let parts = || {
        (fs::read_dir(&cache).into_iter().flatten().flatten())
            .filter(|item| item.file_name().to_string_lossy().ends_with(".part"))
            .count()
    };
    assert_eq!(parts(), 1, "FFmpeg is writing");
    let asked = Instant::now();
    service.cancel(asset.id);
    until("the encoder ending", || !service.busy());
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "cancel waited for the encode"
    );
    until("its process being reaped", || {
        processes.retained_monitors() == 0
    });
    assert_eq!(parts(), 0);
    assert_eq!(service.lookup(&asset), ProxyStatus::None);
    assert!(
        std::iter::from_fn(|| service.poll())
            .all(|event| matches!(event, ProxyEvent::Progress { .. })),
        "a cancelled proxy says neither ready nor failed"
    );
    assert_eq!(service.usage(), (0, 0));
}

#[test]
fn a_video_ffmpeg_cannot_read_fails_with_its_reason() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the unreadable proxy fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let asset = video(&temp.path().join("broken.mp4"), 3840, 2160);
    let cache = temp.path().join("proxies");
    let (service, _) = real(&cache, None, false);
    service.request(&asset).unwrap();
    let message = answer(&service).1.unwrap_err();
    assert!(message.starts_with("ffmpeg"), "{message}");
    assert_eq!(service.lookup(&asset), ProxyStatus::Failed(message));
    assert!(
        fs::read_dir(&cache).unwrap().next().is_none(),
        "no part file is left"
    );
    // Asked again, it is tried again.
    service.request(&asset).unwrap();
    assert_ne!(service.lookup(&asset), ProxyStatus::None);
    let missing = video(&temp.path().join("gone.mp4"), 3840, 2160);
    fs::remove_file(&missing.path).unwrap();
    service.request(&missing).unwrap();
    let (_, gone) = loop {
        let (progress, answer) = answer(&service);
        if answer
            .as_ref()
            .is_err_and(|message| message.contains("gone.mp4"))
        {
            break (progress, answer);
        }
    };
    assert!(gone.unwrap_err().contains("cannot be read"));
}

#[test]
fn a_proxy_seeks_to_the_same_frame_as_a_source_whose_picture_starts_late() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the late picture proxy fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // The sound starts the file; the picture 0.2 s, six frames, after it.
    let source = temp.path().join("late.mkv");
    ffmpeg(
        &[
            "-itsoffset",
            "0.2",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=30:d=3",
            "-f",
            "lavfi",
            "-i",
            "sine=r=48000:d=3.2",
            "-map",
            "0",
            "-map",
            "1",
            "-c:v",
            "ffv1",
            "-c:a",
            "pcm_s16le",
        ],
        &source,
    );
    let asset = imported(&source);
    let (service, _) = real(&temp.path().join("proxies"), None, false);
    service.request(&asset).unwrap();
    let proxy = answer(&service).1.unwrap();
    assert_eq!((proxy.width, proxy.height), (320, 180));
    let rate = asset.frame_rate.unwrap();
    let size = [proxy.width, proxy.height];
    let frame = |path: &Path, target: i64| -> Vec<u8> {
        let mut command = crate::decode::video_command(path, rate, target, size);
        command.args(["-frames:v", "1"]);
        let output = command.output().unwrap();
        assert!(output.status.success());
        let bytes = size[0] as usize * size[1] as usize * 4;
        assert!(output.stdout.len() >= bytes, "a frame at {target}");
        output.stdout[..bytes].to_vec()
    };
    let distance = |a: &[u8], b: &[u8]| -> u64 {
        a.iter()
            .zip(b)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum()
    };
    // The preview decodes the proxy as it decodes the source: at each time
    // the source's own frame is closer than the eight on either side.
    for target in [30, 75] {
        let read = frame(&proxy.path, target);
        let here = distance(&read, &frame(&source, target));
        for other in (target - 8..=target + 8).filter(|other| *other != target) {
            let there = distance(&read, &frame(&source, other));
            assert!(
                here < there,
                "at frame {target} the proxy is nearer the source's frame {other} ({there} < {here})"
            );
        }
    }
}

#[test]
fn pixel_formats_with_alpha_are_known() {
    for format in [
        "yuva420p",
        "yuva444p12le",
        "rgba",
        "bgra",
        "argb",
        "abgr",
        "rgba64le",
        "gbrap10le",
        "gbrapf32le",
        "ya8",
        "ya16le",
        "yaf32le",
        "ayuv64le",
        "vuya",
        "uyva",
        "pal8",
    ] {
        assert!(has_alpha(format), "{format} has alpha");
    }
    for format in [
        "yuv420p",
        "yuvj420p",
        "yuv422p10le",
        "nv12",
        "p010le",
        "rgb24",
        "rgb0",
        "0rgb",
        "bgr0",
        "gbrp",
        "gray",
        "uyvy422",
        "vuyx",
        "",
    ] {
        assert!(!has_alpha(format), "{format} has no alpha");
    }
}

#[test]
fn a_transparent_video_has_no_proxy_and_its_original_is_read() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the transparent proxy fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // ProRes 4444 whose left half is clear.
    let source = temp.path().join("overlay.mov");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=0.5,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(lt(X,W/2),0,255)'",
            "-c:v",
            "prores_ks",
            "-profile:v",
            "4444",
            "-pix_fmt",
            "yuva444p10le",
        ],
        &source,
    );
    assert!(transparent(&source, &Processes::default()));
    let asset = imported(&source);
    let cache = temp.path().join("proxies");
    let (service, processes) = real(&cache, None, false);
    service.request(&asset).unwrap();
    let message = answer(&service).1.unwrap_err();
    assert!(message.contains("transparency"), "{message}");
    assert_eq!(service.lookup(&asset), ProxyStatus::Failed(message));
    assert_eq!(service.usage(), (0, 0));
    let left = (fs::read_dir(&cache).into_iter().flatten().flatten()).count();
    assert_eq!(left, 0, "nothing is written");
    // Only the probe ran: no encoder was started.
    assert_eq!(processes.started_tools(), 1);
    // An opaque one of the same kind is made.
    let opaque = temp.path().join("opaque.mov");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=0.5",
            "-c:v",
            "prores_ks",
            "-profile:v",
            "2",
        ],
        &opaque,
    );
    assert!(!transparent(&opaque, &Processes::default()));
    service.request(&imported(&opaque)).unwrap();
    answer(&service).1.unwrap();
}

#[test]
fn shutdown_stops_the_encoder_removes_its_part_file_and_refuses_more() {
    if !ffmpeg_available() {
        eprintln!("SKIP: FFmpeg is unavailable; the proxy shutdown fixture was not run");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("long.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30",
            "-t",
            "20",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
        ],
        &source,
    );
    let asset = imported(&source);
    let cache = temp.path().join("proxies");
    let (service, processes) = real(&cache, None, true);
    service.request(&asset).unwrap();
    until(
        "progress",
        || matches!(service.lookup(&asset), ProxyStatus::Generating(done) if done > 0.0),
    );
    let parts = || {
        (fs::read_dir(&cache).into_iter().flatten().flatten())
            .filter(|item| item.file_name().to_string_lossy().ends_with(".part"))
            .count()
    };
    assert_eq!(parts(), 1, "FFmpeg is writing");
    let asked = Instant::now();
    service.shutdown();
    assert!(
        asked.elapsed() < SHUTDOWN_GRACE + Duration::from_millis(250),
        "shutdown waits a moment at most"
    );
    // By the time it returns the encoder is reaped and its file gone.
    assert!(!service.busy());
    assert_eq!(parts(), 0, "no part file is left");
    until("its process being reaped", || {
        processes.retained_monitors() == 0
    });
    assert!(matches!(service.request(&asset), Err(MediaError::Closed)));
    assert_eq!(service.poll(), None, "nothing is told after shutdown");
    // Again, or dropped after, waits no more.
    let again = Instant::now();
    service.shutdown();
    drop(service);
    assert!(again.elapsed() < Duration::from_millis(100));
}
