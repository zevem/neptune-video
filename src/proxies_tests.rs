//! Which videos get a proxy, and the lane driven as the window drives it:
//! a real `ProxyService` making a real proxy with FFmpeg. A test that needs
//! FFmpeg and has none prints `SKIP:` and passes.
//!
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the proxies.
use super::*;
use crate::{
    settings::PreviewQuality,
    textures::{Viewing, Wanted},
};
use editor_core::{FrameRate, Time};
use editor_media::{MediaEvent, MediaService};
use std::{
    process::Command as Process,
    sync::Arc,
    time::{Duration, Instant},
};

fn video(name: &str, width: u32, height: u32, seconds: f64, bytes: u64) -> Asset {
    let mut asset = Asset::new(
        PathBuf::from(format!("/media/{name}")),
        MediaKind::Video,
        Time::from_seconds(seconds),
    );
    asset.width = width;
    asset.height = height;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = bytes;
    asset
}

fn project_of(assets: &[Asset]) -> Project {
    let mut project = Project::new("Proxies");
    for asset in assets {
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    }
    project
}

fn names(project: &Project, mode: ProxyMode) -> Vec<String> {
    let mut names: Vec<String> = wanted(project, mode)
        .map(|asset| asset.name.clone())
        .collect();
    names.sort();
    names
}

#[test]
fn each_mode_wants_proxies_of_the_videos_it_says() {
    let uhd = video("uhd.mov", 3840, 2160, 10.0, 400_000_000);
    let hd = video("hd.mp4", 1920, 1080, 10.0, 10_000_000);
    // 80 Mb/s at 1080p: heavy enough.
    let heavy = video("heavy.mov", 1920, 1080, 10.0, 100_000_000);
    let mut still = video("still.png", 6000, 4000, 5.0, 1_000);
    still.kind = MediaKind::Image;
    let mut sound = video("sound.wav", 0, 0, 5.0, 1_000);
    sound.kind = MediaKind::Audio;
    let unknown = video("unknown.mkv", 0, 0, 5.0, 1_000);
    let project = project_of(&[uhd, hd, heavy, still, sound, unknown]);
    assert!(names(&project, ProxyMode::Off).is_empty());
    assert_eq!(names(&project, ProxyMode::Auto), ["heavy.mov", "uhd.mov"]);
    assert_eq!(
        names(&project, ProxyMode::Always),
        ["hd.mp4", "heavy.mov", "uhd.mov"]
    );
}

#[test]
fn a_window_without_the_lane_reads_originals_and_asks_nothing() {
    let asset = video("uhd.mov", 3840, 2160, 10.0, 400_000_000);
    let project = project_of(std::slice::from_ref(&asset));
    let mut proxies = Proxies::none();
    let mut textures = Textures::default();
    let preview = PreviewSettings {
        quality: PreviewQuality::Half,
        ..PreviewSettings::default()
    };
    proxies.replaced(&project, &preview, &mut textures);
    let taken = proxies.take(&project, &preview, &mut textures);
    assert_eq!(taken, Taken::default());
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
    assert_eq!(proxies.cache(), Cache::default());
    assert!(proxies.create(&asset, &mut textures).is_err());
    // The quality is still the person's.
    assert_eq!(textures.detail(), 2);
}

/// FFmpeg as the media crate finds it, when there is one.
fn ffmpeg() -> Option<PathBuf> {
    let path =
        std::env::var_os("ASTER_FFMPEG").map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from);
    Process::new(&path)
        .arg("-version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| path)
}

/// A directory of the test's own, removed however the test ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A second of 2560 by 1440 video at 30 frames a second, imported by the
/// media worker as the application imports it.
fn imported(scratch: &Scratch) -> Option<Asset> {
    imported_from(scratch, "wide.mp4", "testsrc2=size=2560x1440:rate=30")
}

/// Writes a second of the lavfi `source` to `file` with FFmpeg.
fn written(file: &Path, source: &str) -> Option<()> {
    let made = Process::new(ffmpeg()?)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(["-f", "lavfi", "-i", source])
        .args(["-t", "1", "-c:v", "libx264", "-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .arg(file)
        .status()
        .ok()?;
    assert!(made.success(), "FIXTURE: {} is written", file.display());
    Some(())
}

/// A second of the lavfi `source` written to `name`, imported by the media
/// worker as the application imports it.
fn imported_from(scratch: &Scratch, name: &str, source: &str) -> Option<Asset> {
    let file = scratch.0.join(name);
    written(&file, source)?;
    let media = MediaService::new(|| {}).expect("FIXTURE: the media workers start");
    media
        .import(vec![file])
        .expect("FIXTURE: the import is taken");
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        match media.poll() {
            Some(MediaEvent::Imported { assets, errors, .. }) => {
                assert!(errors.is_empty(), "FIXTURE: {errors:?}");
                return assets.into_iter().next();
            }
            Some(_) => {}
            None => {
                assert!(Instant::now() < until, "FIXTURE: the import timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// Takes from the lane until `done` holds of what `textures` was told.
fn until(
    proxies: &mut Proxies,
    project: &Project,
    preview: &PreviewSettings,
    textures: &mut Textures,
    what: &str,
    done: impl Fn(&Textures, &Proxies) -> bool,
) -> Taken {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut taken = Taken::default();
    loop {
        let now = proxies.take(project, preview, textures);
        taken.ready |= now.ready;
        taken.failures.extend(now.failures);
        if done(textures, proxies) {
            return taken;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn source<'a>(asset: &'a Asset, size: [u32; 2]) -> Wanted<'a> {
    Wanted {
        viewing: Viewing::Source,
        asset,
        size,
        time: Time::ZERO,
        carries_on: Vec::new(),
    }
}

#[test]
fn the_lane_makes_reads_keeps_open_removes_and_clears_proxies() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("aster-proxies-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&scratch.0).expect("FIXTURE: a folder for the test");
    let Some(asset) = imported(&scratch) else {
        println!("SKIP: no FFmpeg here to make a proxy with");
        return;
    };
    assert!(
        wants_proxy(&asset),
        "FIXTURE: a video Auto makes a proxy of"
    );
    let project = project_of(std::slice::from_ref(&asset));
    let mut preview = PreviewSettings::default();
    assert_eq!(preview.proxies, ProxyMode::Auto);
    let mut proxies = Proxies::start(|| {}, scratch.0.join("proxies"), &preview);
    assert_eq!(proxies.problem(), None);
    let mut textures = Textures::default();

    // Opening the project asks for what Auto wants, and it is made.
    proxies.replaced(&project, &preview, &mut textures);
    let taken = until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "a proxy",
        |textures, _| matches!(textures.proxy_status(asset.id), ProxyStatus::Ready(_)),
    );
    assert!(taken.ready, "a proxy becoming ready asks for a repaint");
    assert!(taken.failures.is_empty());
    let ProxyStatus::Ready(proxy) = textures.proxy_status(asset.id).clone() else {
        unreachable!()
    };
    assert_eq!([proxy.width, proxy.height], [1280, 720]);
    assert!(proxy.path.starts_with(scratch.0.join("proxies")));
    let cache = proxies.cache();
    assert!(
        cache.available && cache.proxies == 1 && cache.bytes > 0,
        "{cache:?}"
    );

    // The viewer reads it, and the lane is told it is open.
    textures.compose(None, false, &[source(&asset, [960, 540])]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    proxies.take(&project, &preview, &mut textures);
    assert_eq!(proxies.told_open(), std::slice::from_ref(&proxy.path));

    // Off: the original is read again and nothing is open.
    preview.proxies = ProxyMode::Off;
    proxies.mode_changed(&project, &preview, &mut textures);
    textures.compose(None, false, &[source(&asset, [960, 540])]);
    assert_eq!(textures.reads(Viewing::Source), None);
    proxies.take(&project, &preview, &mut textures);
    assert!(proxies.told_open().is_empty());

    // Removed by hand: Auto does not make it again on its own.
    preview.proxies = ProxyMode::Auto;
    proxies.mode_changed(&project, &preview, &mut textures);
    proxies.remove(&asset, &mut textures);
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
    proxies.library_changed(&project, &preview, &mut textures);
    proxies.take(&project, &preview, &mut textures);
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
    until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "the removal",
        |_, proxies| proxies.cache().proxies == 0,
    );

    // Asked for by hand, it is made again.
    proxies
        .create(&asset, &mut textures)
        .expect("a proxy is asked for");
    assert_ne!(textures.proxy_status(asset.id), &ProxyStatus::None);
    until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "the proxy again",
        |textures, _| matches!(textures.proxy_status(asset.id), ProxyStatus::Ready(_)),
    );

    // Cleared: nothing is left, and Auto makes nothing again until asked.
    proxies.clear(&project, &mut textures);
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
    proxies.library_changed(&project, &preview, &mut textures);
    until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "the clearing",
        |_, proxies| proxies.cache().proxies == 0,
    );
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);

    // Another project opened: what Auto wants of it is asked again, and
    // the cache's directory is read for it.
    proxies.replaced(&project, &preview, &mut textures);
    until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "a proxy for the new project",
        |textures, _| matches!(textures.proxy_status(asset.id), ProxyStatus::Ready(_)),
    );
}

#[test]
fn a_proxy_asked_for_by_hand_that_cannot_be_made_is_said() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("aster-proxies-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&scratch.0).expect("FIXTURE: a folder for the test");
    if ffmpeg().is_none() {
        println!("SKIP: no FFmpeg here");
        return;
    }
    // A file that is not there: the lane says so, and is not asked again.
    let mut asset = video("gone.mov", 3840, 2160, 2.0, 1_000_000);
    asset.path = scratch.0.join("gone.mov");
    let project = project_of(std::slice::from_ref(&asset));
    let preview = PreviewSettings {
        proxies: ProxyMode::Off,
        ..PreviewSettings::default()
    };
    let mut proxies = Proxies::start(|| {}, scratch.0.join("proxies"), &preview);
    let mut textures = Textures::default();
    proxies.replaced(&project, &preview, &mut textures);
    assert_eq!(
        textures.proxy_status(asset.id),
        &ProxyStatus::None,
        "Off asks nothing"
    );
    proxies
        .create(&asset, &mut textures)
        .expect("a proxy is asked for");
    let taken = until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "the failure",
        |textures, _| matches!(textures.proxy_status(asset.id), ProxyStatus::Failed(_)),
    );
    assert_eq!(taken.failures.len(), 1, "{:?}", taken.failures);
    assert!(
        taken.failures[0].contains("gone.mov"),
        "{:?}",
        taken.failures
    );
    // The mode does not ask for it again while it has failed.
    let auto = PreviewSettings::default();
    proxies.mode_changed(&project, &auto, &mut textures);
    assert!(matches!(
        textures.proxy_status(asset.id),
        ProxyStatus::Failed(_)
    ));
}

/// The colour at the centre of the first picture of `file`, as RGB.
fn centre(file: &Path, size: [u32; 2]) -> [u8; 3] {
    let output = Process::new(ffmpeg().expect("FIXTURE: FFmpeg is here"))
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(file)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .expect("FIXTURE: FFmpeg runs");
    assert!(
        output.status.success(),
        "FIXTURE: {} decodes",
        file.display()
    );
    assert_eq!(output.stdout.len(), (size[0] * size[1] * 3) as usize);
    let at = (((size[1] / 2) * size[0] + size[0] / 2) * 3) as usize;
    [
        output.stdout[at],
        output.stdout[at + 1],
        output.stdout[at + 2],
    ]
}

fn near(colour: [u8; 3], to: [u8; 3]) -> bool {
    colour.iter().zip(to).all(|(a, b)| a.abs_diff(b) <= 12)
}

#[test]
fn an_export_reads_the_original_of_an_asset_whose_proxy_the_viewer_reads() {
    use crate::export::{
        queue::{CapabilityState, RenderQueue},
        wiring,
    };
    use editor_core::{
        Clip, Editor, TrackKind,
        export::{EncoderChoice, ExportSettings, RenderState, VideoSettings},
    };
    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    let size = [640, 360];

    let scratch =
        Scratch(std::env::temp_dir().join(format!("aster-proxies-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&scratch.0).expect("FIXTURE: a folder for the test");
    let Some(asset) = imported_from(&scratch, "red.mp4", "color=red:size=640x360:rate=30") else {
        println!("SKIP: no FFmpeg here to make a proxy with");
        return;
    };
    let Some(gpu) = crate::compose::gpu_tests::device() else {
        return;
    };

    // A proxy is made of it, and its file is then replaced by one of another
    // colour: whatever reads the proxy shows blue, and the original is red.
    let mut project = project_of(std::slice::from_ref(&asset));
    project.settings.width = size[0];
    project.settings.height = size[1];
    project.settings.frame_rate = FrameRate {
        numerator: 30,
        denominator: 1,
    };
    let track = (project.tracks.iter_mut())
        .find(|track| track.kind == TrackKind::Video)
        .expect("FIXTURE: a new project has a video track");
    Arc::make_mut(track)
        .clips
        .push(Clip::from_asset(&asset, Time::ZERO));
    let editor = Editor::new(project).expect("FIXTURE: the project is valid");
    let project = editor.project();
    let preview = PreviewSettings {
        proxies: ProxyMode::Always,
        ..PreviewSettings::default()
    };
    let mut proxies = Proxies::start(|| {}, scratch.0.join("proxies"), &preview);
    let mut textures = Textures::default();
    proxies.replaced(project, &preview, &mut textures);
    until(
        &mut proxies,
        project,
        &preview,
        &mut textures,
        "a proxy",
        |textures, _| matches!(textures.proxy_status(asset.id), ProxyStatus::Ready(_)),
    );
    let ProxyStatus::Ready(proxy) = textures.proxy_status(asset.id).clone() else {
        unreachable!()
    };
    let extension = proxy
        .path
        .extension()
        .expect("FIXTURE: a proxy file has a type");
    let blue = scratch.0.join("blue").with_extension(extension);
    written(&blue, "color=blue:size=640x360:rate=30").expect("FIXTURE: FFmpeg is here");
    std::fs::rename(&blue, &proxy.path).expect("FIXTURE: the proxy file is replaced");
    assert!(
        near(centre(&proxy.path, [proxy.width, proxy.height]), BLUE),
        "FIXTURE"
    );
    assert!(near(centre(&asset.path, size), RED), "FIXTURE");

    // The viewer reads the proxy.
    textures.compose(None, false, &[source(&asset, size)]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));

    // The project is exported as the window queues it.
    let renders = RenderQueue::new(|| {}, Some((gpu.0.clone(), gpu.1.clone())), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while matches!(renders.capabilities(), CapabilityState::Probing) {
        assert!(Instant::now() < deadline, "the capabilities are known");
        std::thread::sleep(Duration::from_millis(5));
    }
    let settings = ExportSettings {
        video: Some(VideoSettings {
            encoder: EncoderChoice::Software,
            ..VideoSettings::default()
        }),
        audio: None,
        ..ExportSettings::default()
    };
    let out = scratch.0.join("film.mp4");
    let job = wiring::new_job(
        editor.snapshot(),
        editor.instance_id(),
        editor.revision(),
        settings,
        out.clone(),
        true,
        None,
    );
    let id = (renders.enqueue(job)).unwrap_or_else(|error| panic!("the export is queued: {error}"));
    let deadline = Instant::now() + Duration::from_secs(120);
    let info = loop {
        let info = renders.job(id).expect("the job is listed");
        if info.state.is_finished() {
            break info;
        }
        assert!(Instant::now() < deadline, "the export finishes: {info:?}");
        std::thread::sleep(Duration::from_millis(10));
    };
    renders.shutdown();
    assert_eq!(info.state, RenderState::Done, "{:?}", info.message);

    // Its pictures are the original's.
    let exported = centre(&out, size);
    assert!(
        near(exported, RED),
        "the export shows the original: {exported:?}"
    );
    assert!(!near(exported, BLUE), "and never the proxy: {exported:?}");
}

#[test]
fn a_proxy_the_budget_removes_is_no_longer_said_ready_and_the_original_is_read() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("aster-proxies-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&scratch.0).expect("FIXTURE: a folder for the test");
    let Some(first) = imported_from(&scratch, "first.mp4", "testsrc2=size=640x360:rate=30") else {
        println!("SKIP: no FFmpeg here to make a proxy with");
        return;
    };
    let second = imported_from(&scratch, "second.mp4", "testsrc=size=640x360:rate=30")
        .expect("FIXTURE: a second video");
    let project = project_of(&[first.clone(), second.clone()]);
    let preview = PreviewSettings {
        proxies: ProxyMode::Always,
        ..PreviewSettings::default()
    };
    let mut proxies = Proxies::start(|| {}, scratch.0.join("proxies"), &preview);
    let mut textures = Textures::default();
    proxies.replaced(&project, &preview, &mut textures);
    let ready = |textures: &Textures, asset: &Asset| {
        matches!(textures.proxy_status(asset.id), ProxyStatus::Ready(_))
    };
    until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "both proxies",
        |textures, _| ready(textures, &first) && ready(textures, &second),
    );

    // A budget of a byte keeps only the newest proxy; the other is taken
    // out and its file removed.
    (proxies.service.as_ref())
        .expect("FIXTURE: the lane runs")
        .set_budget(1);
    until(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        "the removal",
        |_, proxies| proxies.cache().proxies == 1,
    );
    let (gone, kept) = until_one_is_told(
        &mut proxies,
        &project,
        &preview,
        &mut textures,
        [&first, &second],
    );
    assert!(ready(&textures, kept), "the newest proxy stays");
    // The window says there is none, so the viewer reads the original rather
    // than a deleted file, and nothing is said to have failed.
    assert_eq!(textures.proxy_status(gone.id), &ProxyStatus::None);
    textures.compose(None, false, &[source(gone, [640, 360])]);
    assert_eq!(textures.reads(Viewing::Source), None);
    assert!(!textures.proxy_unreadable(gone.id));
    // The mode does not make it again on its own: that would only push out
    // the other, over and over.
    for _ in 0..10 {
        proxies.take(&project, &preview, &mut textures);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(textures.proxy_status(gone.id), &ProxyStatus::None);
    assert!(ready(&textures, kept));
}

/// Takes from the lane until exactly one of `assets` is no longer said to
/// have a ready proxy; that one, then the other.
fn until_one_is_told<'a>(
    proxies: &mut Proxies,
    project: &Project,
    preview: &PreviewSettings,
    textures: &mut Textures,
    assets: [&'a Asset; 2],
) -> (&'a Asset, &'a Asset) {
    let ready = |textures: &Textures, asset: &Asset| {
        matches!(textures.proxy_status(asset.id), ProxyStatus::Ready(_))
    };
    until(
        proxies,
        project,
        preview,
        textures,
        "the window being told",
        |textures, _| ready(textures, assets[0]) != ready(textures, assets[1]),
    );
    if ready(textures, assets[0]) {
        (assets[1], assets[0])
    } else {
        (assets[0], assets[1])
    }
}
