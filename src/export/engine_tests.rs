//! Exports end to end: generated media, the render queue and its engine, the
//! shared test device and the installed FFmpeg, and the files checked with
//! ffprobe and decoded back.
//!
//! Fixtures are small (320 × 180, one to six seconds) and made with lavfi in
//! a folder of the test's own. A test without FFmpeg says so on stderr and
//! passes; one without a device prints `SKIP:` and passes.
use super::{
    Engine, FINISH_DEADLINE, FINISH_PACE, FINISH_STALL, FinishWatch, Scratch as Files,
    fail_encoder_with, fail_first_encoder, file_problem, part_path, sidecar_for, sidecar_path,
    temp_path, work_path,
};
use crate::{
    compose::gpu_tests::device,
    export::queue::{CapabilityState, NewJob, RenderQueue},
};
use editor_core::{
    Asset, Clip, Element, ElementKind, FrameRate, MediaKind, Project, Time, Track, TrackKind,
    export::{
        AudioCodec, AudioSettings, Capabilities, Container, EncodeSpeed, EncoderChoice,
        ExportRange, ExportSettings, Fit, HardwareKind, RateControl, RenderJobInfo, RenderState,
        Resolution, SidecarFormat, SubtitleSettings, VideoCodec, VideoSettings,
    },
};
use eframe::wgpu;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const RATE: FrameRate = FrameRate {
    numerator: 30,
    denominator: 1,
};
/// The frame the flash fixture is white on, and its sound starts at.
const FLASH: u64 = 20;
const PATIENCE: Duration = Duration::from_secs(180);

fn tool(name: &str) -> Command {
    let variable = if name == "ffmpeg" {
        "NEPTUNE_VIDEO_FFMPEG"
    } else {
        "NEPTUNE_VIDEO_FFPROBE"
    };
    Command::new(std::env::var_os(variable).unwrap_or_else(|| name.into()))
}

fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].into_iter().all(|name| {
        tool(name)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// This host's capabilities, probed once.
fn caps() -> Option<&'static Capabilities> {
    static PROBED: OnceLock<Option<Capabilities>> = OnceLock::new();
    PROBED
        .get_or_init(|| {
            if !ffmpeg_available() {
                return None;
            }
            Some(
                editor_media::export::probe_capabilities()
                    .expect("FFmpeg answers the capability probe"),
            )
        })
        .as_ref()
}

macro_rules! require_ffmpeg {
    ($what:expr) => {
        if caps().is_none() {
            eprintln!("FFmpeg is unavailable; {} skipped", $what);
            return;
        }
    };
}

fn has_encoder(name: &str) -> bool {
    caps().is_some_and(|caps| {
        caps.video.iter().any(|e| e.name == name) || caps.audio.iter().any(|e| e.name == name)
    })
}

/// A folder of the test's own, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("neptune-video-export-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("FIXTURE: the scratch folder is made");
        Self(
            path.canonicalize()
                .expect("FIXTURE: the scratch folder resolves"),
        )
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Hidden part files left in the folder.
    fn parts(&self) -> Vec<String> {
        (fs::read_dir(&self.0).unwrap().flatten())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".part"))
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(command: &mut Command) -> Vec<u8> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .expect("FIXTURE: the tool runs");
    assert!(
        output.status.success(),
        "FIXTURE: {command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn ffmpeg(args: &[&str]) -> Vec<u8> {
    let mut command = tool("ffmpeg");
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    command.args(args);
    run(&mut command)
}

/// Black pictures, white on frame [`FLASH`] alone, and a 1 kHz tone that
/// starts on that frame's first sample: lossless enough to find both again.
fn flash(dir: &Scratch, seconds: u32) -> PathBuf {
    let path = dir.join("flash.mkv");
    let video = format!(
        "color=black:s=320x180:r=30:d={seconds},format=yuv420p,\
         drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='eq(n,{FLASH})'"
    );
    let audio = format!(
        "aevalsrc='if(gte(n,{}),0.5*sin(2*PI*1000*t),0)':s=48000:d={seconds}",
        FLASH * 48_000 / 30
    );
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video,
        "-f",
        "lavfi",
        "-i",
        &audio,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-qp",
        "0",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "15",
        "-c:a",
        "pcm_s16le",
        path.to_str().unwrap(),
    ]);
    path
}

/// A picture of one colour, no sound.
fn plain(dir: &Scratch, colour: &str, size: [u32; 2], seconds: u32) -> PathBuf {
    let path = dir.join(&format!("{colour}-{}x{}.mkv", size[0], size[1]));
    let video = format!(
        "color={colour}:s={}x{}:r=30:d={seconds},format=yuv420p",
        size[0], size[1]
    );
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-qp",
        "0",
        "-g",
        "15",
        path.to_str().unwrap(),
    ]);
    path
}

/// Moving test pictures with a tone, for jobs that should take a moment.
fn pattern(dir: &Scratch, seconds: u32) -> PathBuf {
    let path = dir.join("pattern.mkv");
    let video = format!("testsrc2=s=320x180:r=30:d={seconds}");
    let audio = format!("sine=f=440:r=48000:d={seconds}");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video,
        "-f",
        "lavfi",
        "-i",
        &audio,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "18",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "15",
        "-c:a",
        "pcm_s16le",
        path.to_str().unwrap(),
    ]);
    path
}

fn video_asset(path: &Path, size: [u32; 2], seconds: u32, has_audio: bool) -> Asset {
    let mut asset = Asset::new(
        path.to_path_buf(),
        MediaKind::Video,
        Time::from_seconds(f64::from(seconds)),
    );
    asset.width = size[0];
    asset.height = size[1];
    asset.frame_rate = Some(RATE);
    asset.has_audio = has_audio;
    asset.file_size = fs::metadata(path).map_or(0, |meta| meta.len());
    asset
}

/// A project of `canvas` at 30 fps with `asset` on its video track from the
/// start, as long as the asset.
fn film(canvas: [u32; 2], asset: &Asset) -> Project {
    let mut project = Project::new("Export test");
    project.settings.width = canvas[0];
    project.settings.height = canvas[1];
    project.settings.frame_rate = RATE;
    project.settings.background = [0, 0, 0, 255];
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let mut clip = Clip::from_asset(asset, Time::ZERO);
    clip.duration = asset.duration;
    put(&mut project, TrackKind::Video, clip);
    project
}

/// Puts `clip` on the first track of `kind`.
fn put(project: &mut Project, kind: TrackKind, clip: Clip) {
    let track = (project.tracks.iter_mut())
        .find(|track| track.kind == kind && track.language.is_none())
        .expect("a new project has a track of each kind");
    let track = Arc::make_mut(track);
    track.clips.push(clip);
    track.clips.sort_by_key(|clip| clip.start);
}

/// A caption track in English with one cue.
fn captions(project: &mut Project, text: &str, from: f64, to: f64) {
    let mut track = Track::new("Captions", TrackKind::Text);
    track.language = Some("en".into());
    track.clips.push(Clip::text(
        text,
        Time::from_seconds(from),
        Time::from_seconds(to - from),
    ));
    project.tracks.push(Arc::new(track));
}

fn checked(project: Project) -> Arc<Project> {
    if let Err(error) = project.validate() {
        panic!("FIXTURE: the project is not valid: {error}");
    }
    Arc::new(project)
}

fn mp4(encoder: EncoderChoice) -> ExportSettings {
    ExportSettings {
        container: Container::Mp4,
        video: Some(VideoSettings {
            codec: VideoCodec::H264,
            resolution: Resolution::Project,
            frame_rate: None,
            rate_control: RateControl::default(),
            speed: EncodeSpeed::Faster,
            encoder,
            fit: Fit::Letterbox,
        }),
        audio: Some(AudioSettings::default()),
        range: ExportRange::Timeline,
        subtitles: SubtitleSettings::default(),
    }
}

/// A queue running the real engine on `gpu`, its capabilities known.
fn queue(gpu: Option<(wgpu::Device, wgpu::Queue)>) -> RenderQueue {
    let caps = caps().expect("FFmpeg is available").clone();
    let mut engine = Engine::new(gpu, None);
    let queue = RenderQueue::with_parts(
        || {},
        Arc::new(move || Ok(caps.clone())),
        Box::new(move |ticket, handle| engine.run(ticket, handle)),
    );
    let until = Instant::now() + Duration::from_secs(5);
    while !matches!(queue.capabilities(), CapabilityState::Ready(_)) {
        assert!(Instant::now() < until, "the capabilities are known");
        thread::sleep(Duration::from_millis(2));
    }
    queue
}

fn job(project: &Arc<Project>, settings: ExportSettings, destination: PathBuf) -> NewJob {
    NewJob {
        project: Arc::clone(project),
        context: Uuid::new_v4(),
        revision: 1,
        settings,
        destination,
        start_paused: false,
        project_path: None,
    }
}

/// Waits for a job to finish.
fn finished(queue: &RenderQueue, id: Uuid) -> RenderJobInfo {
    let until = Instant::now() + PATIENCE;
    loop {
        let info = queue.job(id).expect("the job is listed");
        if info.state.is_finished() {
            return info;
        }
        assert!(Instant::now() < until, "the export finishes: {info:?}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Queues `settings` of `project` to `destination` and waits for the file.
fn export(
    queue: &RenderQueue,
    project: &Arc<Project>,
    settings: ExportSettings,
    destination: PathBuf,
) -> RenderJobInfo {
    let id = queue
        .enqueue(job(project, settings, destination.clone()))
        .unwrap_or_else(|error| panic!("the export is queued: {error}"));
    let info = finished(queue, id);
    assert_eq!(
        info.state,
        RenderState::Done,
        "the export is done: {:?}",
        info.message
    );
    assert!(destination.is_file());
    assert!(
        !part_path(&destination, id).exists(),
        "no part file is left"
    );
    assert!(!temp_path(id).exists(), "no working folder is left");
    assert!(
        !work_path(&destination, id).exists(),
        "no working folder is left beside the file"
    );
    assert_eq!(
        info.output_bytes,
        Some(fs::metadata(&destination).unwrap().len())
    );
    info
}

fn probe(path: &Path, entries: &str) -> Value {
    let mut command = tool("ffprobe");
    command
        .args(["-v", "error", "-show_entries", entries, "-of", "json"])
        .arg(path);
    serde_json::from_slice(&run(&mut command)).unwrap()
}

fn stream<'a>(probe: &'a Value, kind: &str) -> Option<&'a Value> {
    probe["streams"]
        .as_array()?
        .iter()
        .find(|stream| stream["codec_type"] == kind)
}

fn streams(path: &Path) -> Value {
    probe(
        path,
        "stream=codec_type,codec_name,width,height,sample_rate,channels:format=duration",
    )
}

fn duration(probe: &Value) -> f64 {
    probe["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .expect("the file has a duration")
}

/// Pictures in the file, counted by decoding.
fn frame_count(path: &Path) -> u64 {
    let mut command = tool("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(path);
    String::from_utf8_lossy(&run(&mut command))
        .trim()
        .trim_end_matches(',')
        .parse()
        .expect("ffprobe counts the frames")
}

/// Each picture's mean brightness.
fn brightness(path: &Path, size: [u32; 2]) -> Vec<f64> {
    let bytes = ffmpeg(&[
        "-i",
        path.to_str().unwrap(),
        "-map",
        "0:v:0",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "gray",
        "pipe:1",
    ]);
    let frame = (size[0] * size[1]) as usize;
    bytes
        .chunks_exact(frame)
        .map(|picture| picture.iter().map(|&v| f64::from(v)).sum::<f64>() / frame as f64)
        .collect()
}

/// Picture `index` as RGB rows.
fn picture(path: &Path, index: u64, size: [u32; 2]) -> Vec<u8> {
    let select = format!("select=eq(n\\,{index})");
    let bytes = ffmpeg(&[
        "-i",
        path.to_str().unwrap(),
        "-map",
        "0:v:0",
        "-vf",
        &select,
        "-frames:v",
        "1",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
    ]);
    assert_eq!(bytes.len(), (size[0] * size[1] * 3) as usize);
    bytes
}

fn rgb(picture: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * width + x) * 3) as usize;
    [picture[at], picture[at + 1], picture[at + 2]]
}

/// The first sample of the sound, mixed to one channel.
fn samples(path: &Path) -> Vec<f32> {
    let bytes = ffmpeg(&[
        "-i",
        path.to_str().unwrap(),
        "-map",
        "0:a:0",
        "-ac",
        "1",
        "-f",
        "f32le",
        "pipe:1",
    ]);
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn onset(samples: &[f32]) -> usize {
    samples
        .iter()
        .position(|sample| sample.abs() > 0.1)
        .expect("the tone is heard")
}

/// Processes whose command line mentions `marker`.
#[cfg(target_os = "linux")]
fn processes_mentioning(marker: &str) -> usize {
    let me = std::process::id().to_string();
    (fs::read_dir("/proc").unwrap().flatten())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.chars().all(|c| c.is_ascii_digit()) && name != me
        })
        .filter(|entry| {
            fs::read(entry.path().join("cmdline"))
                .is_ok_and(|line| String::from_utf8_lossy(&line).contains(marker))
        })
        .count()
}

fn gpu(turn: &(wgpu::Device, wgpu::Queue)) -> Option<(wgpu::Device, wgpu::Queue)> {
    Some((turn.0.clone(), turn.1.clone()))
}

#[test]
fn an_mp4_with_sound_has_every_frame_and_its_length() {
    require_ffmpeg!("the MP4 export");
    let Some(device) = device() else { return };
    let dir = Scratch::new("mp4");
    let source = flash(&dir, 2);
    let asset = video_asset(&source, [320, 180], 2, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));
    let out = dir.join("film.mp4");
    let started = Instant::now();
    let info = export(&queue, &project, mp4(EncoderChoice::Auto), out.clone());
    eprintln!(
        "320x180 MP4 ({}): 60 frames in {:.2}s",
        info.encoder.as_deref().unwrap_or("?"),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(info.frames_total, 60);
    assert_eq!(info.frames_done, 60);
    assert_eq!(info.progress, 1.0);
    assert_eq!(info.attempts, 1);
    assert!(info.elapsed_seconds > 0.0);
    let found = streams(&out);
    let video = stream(&found, "video").expect("a picture stream");
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(
        (video["width"].as_u64(), video["height"].as_u64()),
        (Some(320), Some(180))
    );
    let audio = stream(&found, "audio").expect("a sound stream");
    assert_eq!(audio["codec_name"], "aac");
    assert_eq!(audio["sample_rate"], "48000");
    assert_eq!(audio["channels"], 2);
    assert!(
        (duration(&found) - 2.0).abs() < 0.06,
        "{}",
        duration(&found)
    );
    assert_eq!(frame_count(&out), 60);
    assert!(dir.parts().is_empty());
}

#[test]
fn a_flash_and_its_tone_land_on_the_same_frame() {
    require_ffmpeg!("the sync export");
    let Some(device) = device() else { return };
    let dir = Scratch::new("sync");
    let source = flash(&dir, 2);
    let asset = video_asset(&source, [320, 180], 2, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));
    let out = dir.join("sync.mp4");
    export(&queue, &project, mp4(EncoderChoice::Software), out.clone());
    let light = brightness(&out, [320, 180]);
    assert_eq!(light.len(), 60);
    let bright = (light.iter().enumerate())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(index, _)| index)
        .unwrap();
    assert_eq!(bright as u64, FLASH, "the white frame stays where it was");
    assert!(light[bright] > 200.0 && light[bright - 1] < 30.0);
    let heard = onset(&samples(&out)) as f64 / 48_000.0;
    let seen = bright as f64 / 30.0;
    assert!(
        (heard - seen).abs() < 1.0 / 30.0,
        "the tone starts at {heard:.4}s and the flash shows at {seen:.4}s"
    );
}

#[test]
fn a_range_starts_its_pictures_and_sound_at_its_start() {
    require_ffmpeg!("the range export");
    let Some(device) = device() else { return };
    let dir = Scratch::new("range");
    let source = flash(&dir, 2);
    let asset = video_asset(&source, [320, 180], 2, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));
    let out = dir.join("range.mp4");
    let mut settings = mp4(EncoderChoice::Software);
    settings.range = ExportRange::Span {
        start: Time::from_seconds(0.5),
        end: Time::from_seconds(1.5),
    };
    let info = export(&queue, &project, settings, out.clone());
    assert_eq!(info.frames_total, 30);
    assert_eq!(frame_count(&out), 30);
    let found = streams(&out);
    assert!(
        (duration(&found) - 1.0).abs() < 0.06,
        "{}",
        duration(&found)
    );
    let light = brightness(&out, [320, 180]);
    let bright = (light.iter().enumerate())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(index, _)| index)
        .unwrap();
    assert_eq!(bright as u64, FLASH - 15, "frame 20 is the range's fifth");
    let heard = onset(&samples(&out)) as f64 / 48_000.0;
    assert!(
        (heard - bright as f64 / 30.0).abs() < 1.0 / 30.0,
        "the tone starts at {heard:.4}s in the range"
    );
}

#[test]
fn sound_only_exports_hold_every_sample() {
    require_ffmpeg!("the sound-only exports");
    let dir = Scratch::new("audio");
    let source = flash(&dir, 2);
    let asset = video_asset(&source, [320, 180], 2, true);
    let project = checked(film([320, 180], &asset));
    // No device: sound alone draws nothing.
    let queue = queue(None);

    let wav = dir.join("sound.wav");
    let settings = ExportSettings {
        container: Container::Wav,
        video: None,
        audio: Some(AudioSettings {
            codec: AudioCodec::Pcm24,
            kbps: 0,
            sample_rate: 48_000,
            channels: 2,
        }),
        ..ExportSettings::default()
    };
    let info = export(&queue, &project, settings, wav.clone());
    assert_eq!(info.frames_total, 0);
    assert_eq!(info.encoder.as_deref(), Some("pcm_s24le"));
    let found = streams(&wav);
    assert!(stream(&found, "video").is_none());
    assert_eq!(stream(&found, "audio").unwrap()["codec_name"], "pcm_s24le");
    let heard = samples(&wav);
    assert_eq!(heard.len(), 96_000, "exactly two seconds of samples");
    let start = onset(&heard);
    assert!(
        start.abs_diff(32_000) <= 1,
        "the tone starts on sample {start}"
    );

    if has_encoder("aac") {
        let m4a = dir.join("sound.m4a");
        let settings = ExportSettings {
            container: Container::M4a,
            video: None,
            audio: Some(AudioSettings::default()),
            ..ExportSettings::default()
        };
        export(&queue, &project, settings, m4a.clone());
        let found = streams(&m4a);
        assert!(stream(&found, "video").is_none());
        assert_eq!(stream(&found, "audio").unwrap()["codec_name"], "aac");
        assert!((duration(&found) - 2.0).abs() < 0.06);
        let heard = samples(&m4a);
        assert!(heard.len().abs_diff(96_000) <= 2048, "{}", heard.len());
    }
}

#[test]
fn webm_and_prores_masters_are_written() {
    require_ffmpeg!("the WebM and MOV exports");
    let Some(device) = device() else { return };
    let dir = Scratch::new("formats");
    let source = flash(&dir, 1);
    let asset = video_asset(&source, [320, 180], 1, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));

    if has_encoder("libvpx-vp9") && has_encoder("libopus") {
        let out = dir.join("web.webm");
        let mut settings = mp4(EncoderChoice::Auto);
        settings.container = Container::Webm;
        settings.video.as_mut().unwrap().codec = VideoCodec::Vp9;
        settings.audio = Some(AudioSettings {
            codec: AudioCodec::Opus,
            kbps: 128,
            sample_rate: 48_000,
            channels: 2,
        });
        export(&queue, &project, settings, out.clone());
        let found = streams(&out);
        assert_eq!(stream(&found, "video").unwrap()["codec_name"], "vp9");
        assert_eq!(stream(&found, "audio").unwrap()["codec_name"], "opus");
        assert_eq!(frame_count(&out), 30);
    } else {
        eprintln!("VP9 or Opus is unavailable; the WebM export skipped");
    }

    if has_encoder("prores_ks") {
        let out = dir.join("master.mov");
        let mut settings = mp4(EncoderChoice::Auto);
        settings.container = Container::Mov;
        settings.video.as_mut().unwrap().codec = VideoCodec::ProRes;
        settings.audio = Some(AudioSettings {
            codec: AudioCodec::Pcm24,
            kbps: 0,
            sample_rate: 48_000,
            channels: 2,
        });
        export(&queue, &project, settings, out.clone());
        let found = streams(&out);
        assert_eq!(stream(&found, "video").unwrap()["codec_name"], "prores");
        assert_eq!(stream(&found, "audio").unwrap()["codec_name"], "pcm_s24le");
        assert_eq!(frame_count(&out), 30);
    } else {
        eprintln!("ProRes is unavailable; the MOV export skipped");
    }
}

#[test]
fn titles_are_burned_in_and_hidden_captions_are_not() {
    require_ffmpeg!("the title export");
    let Some(device) = device() else { return };
    let dir = Scratch::new("titles");
    let source = plain(&dir, "black", [320, 180], 1);
    let asset = video_asset(&source, [320, 180], 1, false);
    let queue = queue(gpu(&device));
    let mut settings = mp4(EncoderChoice::Software);
    settings.audio = None;

    // A red box, half the canvas, in the middle.
    let mut titled = film([320, 180], &asset);
    let red = Element::new(
        "Box",
        ElementKind::Rect {
            color: [230, 30, 30, 255],
            radius: 0.0,
            stroke: None,
        },
        [0.0, 0.0, 160.0, 90.0],
    );
    put(
        &mut titled,
        TrackKind::Text,
        Clip::graphic(
            "Box",
            [160.0, 90.0],
            vec![red],
            Time::ZERO,
            Time::from_seconds(1.0),
        ),
    );
    let titled = checked(titled);
    let out = dir.join("titled.mp4");
    export(&queue, &titled, settings.clone(), out.clone());
    let shown = picture(&out, 10, [320, 180]);
    let [r, g, b] = rgb(&shown, 320, 160, 90);
    assert!(
        r > 180 && g < 70 && b < 70,
        "the title is drawn: {:?}",
        [r, g, b]
    );
    let [r, g, b] = rgb(&shown, 320, 20, 20);
    assert!(
        r < 20 && g < 20 && b < 20,
        "around it is black: {:?}",
        [r, g, b]
    );

    // A caption burned in, and the same caption hidden.
    let mut captioned = film([320, 180], &asset);
    captions(&mut captioned, "HELLO", 0.0, 1.0);
    let captioned = checked(captioned);
    let burned = dir.join("burned.mp4");
    export(&queue, &captioned, settings.clone(), burned.clone());
    let light = |path: &Path| {
        picture(path, 10, [320, 180])
            .chunks_exact(3)
            .map(|p| p.iter().copied().max().unwrap())
            .max()
            .unwrap()
    };
    assert!(light(&burned) > 150, "the caption is drawn");
    let mut hidden = settings;
    hidden.subtitles.burn_in = false;
    let clean = dir.join("clean.mp4");
    export(&queue, &captioned, hidden, clean.clone());
    assert!(light(&clean) < 20, "the caption is not drawn");
}

#[test]
fn cancel_mid_render_leaves_nothing_behind() {
    require_ffmpeg!("the cancelled export");
    let Some(device) = device() else { return };
    let dir = Scratch::new("cancel");
    let source = pattern(&dir, 6);
    let asset = video_asset(&source, [320, 180], 6, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));
    let out = dir.join("cancelled.mp4");
    let mut settings = mp4(EncoderChoice::Software);
    settings.video.as_mut().unwrap().resolution = Resolution::P720;
    let id = queue.enqueue(job(&project, settings, out.clone())).unwrap();
    let until = Instant::now() + PATIENCE;
    loop {
        let info = queue.job(id).unwrap();
        assert!(!info.state.is_finished(), "cancelled in time: {info:?}");
        if info.frames_done >= 5 {
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(queue.cancel(id));
    let info = finished(&queue, id);
    assert_eq!(info.state, RenderState::Cancelled);
    assert!(info.frames_done < info.frames_total);
    assert_eq!(info.message, None);
    assert!(!out.exists());
    assert!(dir.parts().is_empty(), "no part file: {:?}", dir.parts());
    assert!(!temp_path(id).exists());
    assert_eq!(queue.poll(), None);
    #[cfg(target_os = "linux")]
    {
        let marker = dir.0.to_string_lossy().into_owned();
        let until = Instant::now() + Duration::from_secs(5);
        while processes_mentioning(&marker) > 0 {
            assert!(Instant::now() < until, "the job's FFmpeg processes end");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn a_failed_hardware_encode_finishes_in_software() {
    require_ffmpeg!("the hardware fallback");
    if !has_encoder("h264_nvenc") {
        eprintln!("NVENC is unavailable; the hardware fallback skipped");
        return;
    }
    let Some(device) = device() else { return };
    let dir = Scratch::new("fallback");
    let source = flash(&dir, 1);
    let asset = video_asset(&source, [320, 180], 1, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));

    let out = dir.join("fallback.mp4");
    fail_first_encoder(&out);
    let info = export(&queue, &project, mp4(EncoderChoice::Auto), out.clone());
    assert_eq!(info.encoder.as_deref(), Some("libx264"));
    let message = info.message.unwrap_or_default();
    assert!(
        message.starts_with("NVIDIA NVENC failed (")
            && message.contains("finished with libx264 software encoding"),
        "{message}"
    );
    assert_eq!(frame_count(&out), 30);
    match queue.poll() {
        Some(crate::export::queue::RenderEvent::Finished { result, note, .. }) => {
            assert_eq!(result, Ok(out.clone()));
            assert!(note.is_some_and(|note| note.contains("NVENC")));
        }
        other => panic!("expected the finished file, got {other:?}"),
    }

    // Asked for NVENC by name, the failure is the job's.
    let named = dir.join("named.mp4");
    fail_first_encoder(&named);
    let id = queue
        .enqueue(job(
            &project,
            mp4(EncoderChoice::Hardware(HardwareKind::Nvenc)),
            named.clone(),
        ))
        .unwrap();
    let info = finished(&queue, id);
    assert_eq!(info.state, RenderState::Failed);
    assert!(
        info.message
            .as_deref()
            .is_some_and(|m| m.contains("made to fail")),
        "{:?}",
        info.message
    );
    assert!(!named.exists());
    assert!(dir.parts().is_empty());
}

#[test]
fn a_full_disk_fails_at_once_and_two_failures_are_both_told() {
    require_ffmpeg!("the failures that do not fall back");
    if !has_encoder("h264_nvenc") {
        eprintln!("NVENC is unavailable; the failures without fallback skipped");
        return;
    }
    let Some(device) = device() else { return };
    let dir = Scratch::new("no-fallback");
    let source = flash(&dir, 1);
    let asset = video_asset(&source, [320, 180], 1, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));

    // No room on the disk: encoding again in software would meet it again.
    let full = dir.join("full.mp4");
    fail_encoder_with(
        &full,
        "ffmpeg: Encoding stopped after 3 frames: [mp4 @ 0x1] Error writing: No space \
         left on device",
    );
    let id = queue
        .enqueue(job(&project, mp4(EncoderChoice::Auto), full.clone()))
        .unwrap();
    let info = finished(&queue, id);
    assert_eq!(info.state, RenderState::Failed);
    let message = info.message.unwrap_or_default();
    assert!(message.contains("No space left on device"), "{message}");
    assert_eq!(
        info.encoder.as_deref(),
        Some("h264_nvenc"),
        "no software encode was started"
    );
    assert!(!full.exists());
    assert!(dir.parts().is_empty());

    // The hardware encoder fails, then the software one: both are said.
    let twice = dir.join("twice.mp4");
    fail_encoder_with(
        &twice,
        "ffmpeg: [h264_nvenc @ 0x1] OpenEncodeSessionEx failed",
    );
    fail_encoder_with(&twice, "ffmpeg: [libx264 @ 0x1] broke too");
    let id = queue
        .enqueue(job(&project, mp4(EncoderChoice::Auto), twice.clone()))
        .unwrap();
    let info = finished(&queue, id);
    assert_eq!(info.state, RenderState::Failed);
    let message = info.message.unwrap_or_default();
    assert!(
        message.starts_with("NVIDIA NVENC failed (ffmpeg: [h264_nvenc @ 0x1] OpenEncodeSessionEx")
            && message
                .contains("and so did the software encoder: ffmpeg: [libx264 @ 0x1] broke too"),
        "{message}"
    );
    assert!(!twice.exists());
    assert!(dir.parts().is_empty());
}

#[test]
fn a_paused_export_resumes_and_finishes_every_frame() {
    require_ffmpeg!("the paused export");
    let Some(device) = device() else { return };
    let dir = Scratch::new("pause");
    let source = pattern(&dir, 4);
    let asset = video_asset(&source, [320, 180], 4, true);
    let project = checked(film([320, 180], &asset));
    let queue = queue(gpu(&device));
    let out = dir.join("paused.mp4");
    let mut settings = mp4(EncoderChoice::Software);
    settings.video.as_mut().unwrap().resolution = Resolution::Custom {
        width: 640,
        height: 360,
    };
    let id = queue.enqueue(job(&project, settings, out.clone())).unwrap();
    let until = Instant::now() + PATIENCE;
    while queue.job(id).unwrap().frames_done < 3 {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(queue.pause(id));
    // Frames already drawn may still be counted; then nothing moves.
    thread::sleep(Duration::from_millis(100));
    let held = queue.job(id).unwrap();
    assert_eq!(held.state, RenderState::Paused);
    assert!(held.frames_done < held.frames_total);
    thread::sleep(Duration::from_millis(400));
    let later = queue.job(id).unwrap();
    assert_eq!(
        later.frames_done, held.frames_done,
        "nothing is drawn while paused"
    );
    assert!(
        (later.elapsed_seconds - held.elapsed_seconds).abs() < 1e-9,
        "time paused is not counted"
    );
    assert!(queue.summary().paused);
    assert!(queue.resume(id));
    let info = finished(&queue, id);
    assert_eq!(info.state, RenderState::Done, "{:?}", info.message);
    assert_eq!(frame_count(&out), 120);
}

#[test]
fn a_caption_file_is_written_beside_a_range_with_its_times_moved() {
    require_ffmpeg!("the caption file");
    let Some(device) = device() else { return };
    let dir = Scratch::new("sidecar");
    let source = plain(&dir, "black", [320, 180], 2);
    let asset = video_asset(&source, [320, 180], 2, false);
    let mut project = film([320, 180], &asset);
    captions(&mut project, "Hello there", 1.0, 1.5);
    let project = checked(project);
    let queue = queue(gpu(&device));
    let out = dir.join("talk.mp4");
    let mut settings = mp4(EncoderChoice::Software);
    settings.audio = None;
    settings.range = ExportRange::Span {
        start: Time::from_seconds(0.5),
        end: Time::from_seconds(2.0),
    };
    settings.subtitles = SubtitleSettings {
        burn_in: false,
        sidecar: Some(SidecarFormat::Srt),
        embed: true,
        track: None,
    };
    export(&queue, &project, settings, out.clone());
    let sidecar = dir.join("talk.en.srt");
    assert_eq!(sidecar_path(&out, Some("en"), SidecarFormat::Srt), sidecar);
    let text = fs::read_to_string(&sidecar).expect("the caption file is beside the export");
    assert_eq!(text, "1\n00:00:00,500 --> 00:00:01,000\nHello there\n\n");
    let found = streams(&out);
    assert_eq!(
        stream(&found, "subtitle").expect("the captions are carried")["codec_name"],
        "mov_text"
    );
    assert!(dir.parts().is_empty());
}

#[test]
fn a_small_canvas_is_drawn_large_and_another_aspect_letterboxed() {
    require_ffmpeg!("the scaled exports");
    let Some(device) = device() else { return };
    let dir = Scratch::new("sizes");
    let queue = queue(gpu(&device));

    let source = flash(&dir, 2);
    let asset = video_asset(&source, [320, 180], 2, true);
    let project = checked(film([320, 180], &asset));
    let large = dir.join("large.mp4");
    let mut settings = mp4(EncoderChoice::Auto);
    settings.video.as_mut().unwrap().resolution = Resolution::P1080;
    let started = Instant::now();
    let info = export(&queue, &project, settings, large.clone());
    eprintln!(
        "1920x1080 MP4 ({}): 60 frames in {:.2}s",
        info.encoder.as_deref().unwrap_or("?"),
        started.elapsed().as_secs_f64()
    );
    let found = streams(&large);
    let video = stream(&found, "video").unwrap();
    assert_eq!(
        (video["width"].as_u64(), video["height"].as_u64()),
        (Some(1920), Some(1080))
    );
    assert_eq!(frame_count(&large), 60);
    let flashed = picture(&large, FLASH, [1920, 1080]);
    assert!(rgb(&flashed, 1920, 960, 540).iter().all(|&c| c > 230));

    // A 4:3 white canvas in a 16:9 picture: bars left and right.
    let white = plain(&dir, "white", [240, 180], 1);
    let asset = video_asset(&white, [240, 180], 1, false);
    let project = checked(film([240, 180], &asset));
    let boxed = dir.join("boxed.mp4");
    let mut settings = mp4(EncoderChoice::Software);
    settings.audio = None;
    settings.video.as_mut().unwrap().resolution = Resolution::Custom {
        width: 320,
        height: 180,
    };
    export(&queue, &project, settings, boxed.clone());
    let shown = picture(&boxed, 5, [320, 180]);
    for x in [4, 30, 290, 315] {
        let pixel = rgb(&shown, 320, x, 90);
        assert!(pixel.iter().all(|&c| c < 20), "a bar at {x}: {pixel:?}");
    }
    for x in [50, 160, 270] {
        let pixel = rgb(&shown, 320, x, 90);
        assert!(
            pixel.iter().all(|&c| c > 230),
            "the canvas at {x}: {pixel:?}"
        );
    }
}

#[test]
fn a_job_whose_folder_went_fails_and_says_so() {
    require_ffmpeg!("the missing folder");
    let dir = Scratch::new("gone");
    let source = flash(&dir, 1);
    let asset = video_asset(&source, [320, 180], 1, true);
    let project = checked(film([320, 180], &asset));
    let folder = dir.join("out");
    fs::create_dir(&folder).unwrap();
    let queue = queue(None);
    let settings = ExportSettings {
        container: Container::Wav,
        video: None,
        audio: Some(AudioSettings {
            codec: AudioCodec::Pcm16,
            kbps: 0,
            sample_rate: 48_000,
            channels: 2,
        }),
        ..ExportSettings::default()
    };
    let id = queue
        .enqueue(NewJob {
            start_paused: true,
            ..job(&project, settings, folder.join("x.wav"))
        })
        .unwrap();
    fs::remove_dir(&folder).unwrap();
    assert!(queue.resume(id));
    let info = finished(&queue, id);
    assert_eq!(info.state, RenderState::Failed);
    let message = info.message.unwrap_or_default();
    assert_eq!(message, "The destination folder does not exist");
}

// ---------------------------------------------------------------------------
// Without FFmpeg: the finish's patience, failures that are the disk's, and
// the files a job makes
// ---------------------------------------------------------------------------

#[test]
fn a_finish_is_waited_for_while_the_file_changes() {
    let start = Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    let file = |bytes: u64| Some((bytes, None));
    // A file of 80 GB: the floor is 600 s plus 80 GB at 8 MB/s.
    let big = 80 * 1024 * 1024 * 1024;
    let mut watch = FinishWatch::new(start, file(big));
    let floor = FINISH_DEADLINE.as_secs() + big / FINISH_PACE;
    assert!(floor > 10_000);
    assert!(
        watch.patient(at(floor - 1), file(big)),
        "nothing changed, but the floor is not reached"
    );
    // Moving the index to the front rewrites the file for hours: as long
    // as it changes, the wait goes on.
    let mut now = floor;
    for step in 0..100 {
        now += FINISH_STALL.as_secs() - 1;
        let seen = Some((
            big + 4096,
            Some(std::time::UNIX_EPOCH + Duration::from_secs(step)),
        ));
        assert!(watch.patient(at(now), seen), "step {step} is waited for");
    }
    // Then it stops changing: past the stall, the finish has stalled.
    let last = Some((
        big + 4096,
        Some(std::time::UNIX_EPOCH + Duration::from_secs(99)),
    ));
    assert!(watch.patient(at(now + FINISH_STALL.as_secs() - 1), last));
    assert!(!watch.patient(at(now + FINISH_STALL.as_secs()), last));

    // A small file that stops changing is given the floor, then the stall.
    let mut small = FinishWatch::new(start, file(1_000));
    assert!(small.patient(at(FINISH_DEADLINE.as_secs() - 1), file(1_000)));
    assert!(!small.patient(at(FINISH_DEADLINE.as_secs()), file(1_000)));
    // One that has no file yet is given the same.
    let mut none = FinishWatch::new(start, None);
    assert!(none.patient(at(FINISH_DEADLINE.as_secs() - 1), None));
    assert!(!none.patient(at(FINISH_DEADLINE.as_secs() + 1), None));
}

#[test]
fn failures_of_the_disk_or_folder_are_not_the_encoders() {
    let part = Path::new("/films/.Film.mp4.0b7c0f6e-0000-4000-8000-000000000000.part");
    for why in [
        "ffmpeg: Encoding failed (exit status: 1): [mp4 @ 0x1] Error writing trailer: \
         No space left on device",
        "ffmpeg: Encoding stopped after 900 frames: av_interleaved_write_frame(): Disk \
         quota exceeded",
        "ffmpeg: [out#0/mp4 @ 0x1] Error opening output \
         /films/.Film.mp4.0b7c0f6e-0000-4000-8000-000000000000.part: Permission denied",
        "ffmpeg: Error opening output files: No such file or directory\n\
         [out#0/mp4 @ 0x1] Error opening output /gone/x.part: No such file or directory",
        "ffmpeg: /films/.Film.mp4.0b7c0f6e-0000-4000-8000-000000000000.part: File name too \
         long",
        "ffmpeg: Error writing: Read-only file system",
    ] {
        assert!(file_problem(why, part), "{why}");
    }
    for why in [
        "ffmpeg: [h264_nvenc @ 0x1] OpenEncodeSessionEx failed: out of memory (10)",
        "ffmpeg: [AVHWDeviceContext @ 0x1] Failed to open /dev/dri/renderD128: Permission \
         denied\nDevice creation failed: -13.",
        "ffmpeg: [AVHWDeviceContext @ 0x1] No VA display found for device \
         /dev/dri/renderD128: No such file or directory",
        "ffmpeg: the encoder was made to fail for a test",
    ] {
        assert!(!file_problem(why, part), "{why}");
    }
}

#[test]
fn a_long_name_keeps_its_part_file_within_the_name_limit() {
    let id = Uuid::new_v4();
    let folder = Path::new("/films");
    let short = part_path(&folder.join("Film.mp4"), id);
    assert_eq!(
        short.file_name().unwrap().to_string_lossy(),
        format!(".Film.mp4.{id}.part")
    );
    for name in [
        format!("{}.mp4", "a".repeat(250)),
        // Several bytes a character: cut where a character starts.
        format!("{}.mp4", "é".repeat(125)),
        format!("{}.mp4", "🎬".repeat(62)),
    ] {
        assert!(name.len() <= 255, "the destination name itself fits");
        let part = part_path(&folder.join(&name), id);
        let part = part.file_name().unwrap().to_string_lossy().into_owned();
        assert!(part.len() <= 255, "{} bytes: {part}", part.len());
        assert!(part.starts_with('.') && part.ends_with(&format!(".{id}.part")));
        assert_eq!(part_path(&folder.join(&name), id).parent(), Some(folder));
    }
    // The caption file's part is kept short too.
    let caption = sidecar_path(
        &folder.join(format!("{}.mp4", "b".repeat(240))),
        Some("en"),
        SidecarFormat::Srt,
    );
    let part = part_path(&caption, id);
    assert!(part.file_name().unwrap().len() <= 255);
}

#[test]
fn the_working_folder_is_beside_the_file_unless_that_folder_takes_none() {
    let dir = Scratch::new("work");
    let id = Uuid::new_v4();
    let destination = dir.join("Film.mp4");
    {
        let mut files = Files::new(id, &destination);
        files.make_temp().unwrap();
        assert_eq!(files.temp, work_path(&destination, id));
        assert!(files.temp.is_dir());
        assert!(
            files
                .temp
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".neptune-video-render-"),
            "it is hidden"
        );
        fs::write(files.temp.join("sound.f32"), b"sound").unwrap();
    }
    assert!(
        !work_path(&destination, id).exists(),
        "it goes with the job"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let destination = locked.join("Film.mp4");
        if fs::create_dir(locked.join("probe")).is_ok() {
            eprintln!("SKIP: this user writes into read-only folders");
        } else {
            let id = Uuid::new_v4();
            let mut files = Files::new(id, &destination);
            files.make_temp().unwrap();
            assert_eq!(files.temp, temp_path(id), "the temporary folder instead");
            assert!(files.temp.is_dir());
            drop(files);
            assert!(!temp_path(id).exists());
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn the_caption_file_is_named_from_the_track_the_plan_takes() {
    let mut project = Project::new("Captions");
    let destination = Path::new("/films/Talk.mp4");
    let mut settings = ExportSettings::default();
    assert_eq!(sidecar_for(&project, &settings, destination), None);
    settings.subtitles.sidecar = Some(SidecarFormat::Vtt);
    assert_eq!(
        sidecar_for(&project, &settings, destination),
        None,
        "no caption track: the plan refuses"
    );
    captions(&mut project, "Hello", 0.0, 1.0);
    let mut german = Track::new("Untertitel", TrackKind::Text);
    german.language = Some("de".into());
    german
        .clips
        .push(Clip::text("Hallo", Time::ZERO, Time::from_seconds(1.0)));
    let german_id = german.id;
    project.tracks.push(Arc::new(german));
    assert_eq!(
        sidecar_for(&project, &settings, destination),
        Some(PathBuf::from("/films/Talk.en.vtt"))
    );
    settings.subtitles.track = Some(german_id);
    assert_eq!(
        sidecar_for(&project, &settings, destination),
        Some(PathBuf::from("/films/Talk.de.vtt"))
    );
}
